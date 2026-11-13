#![cfg(target_os = "linux")]

#[path = "../../../../fixtures/runtime/custody.rs"]
mod fixture;
use ag1_core::{
    DomainError,
    inference::{EndpointKind, Scheduler},
};
use ag1_platform::{
    cutover::Registry,
    llama::custody::RuntimeBundle,
    provider::{
        generation::{GenerationLimits, GenerationRequest, request_generation},
        http::{
            GenerationRuntime, HostHeaders, HttpMethod, HttpRequest, ProviderHttp, cancellation,
        },
    },
};
use fixture::*;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
fn request(model: &str) -> HttpRequest {
    HttpRequest {
        url: "http://127.0.0.1:18087/v1/chat/completions".into(),
        method: HttpMethod::Post,
        headers: HostHeaders::default(),
        body: serde_json::to_vec(&serde_json::json!({"model":model,"messages":[]})).unwrap(),
        max_response_bytes: 1024,
    }
}
#[tokio::test]
async fn three_models_start_sequentially_with_exact_plans_and_one_runtime_lease() {
    let d = tempfile::tempdir().unwrap();
    let registry = Registry::at(&d.path().join("authority")).unwrap();
    let plans = plans(d.path(), 18087, &["one", "two", "three"], 32768);
    let expected = plans
        .iter()
        .map(|p| (p.command.clone(), p.args.clone(), p.environment.clone()))
        .collect::<Vec<_>>();
    let audit = Arc::new(Mutex::new(Audit::default()));
    let bundle = RuntimeBundle::new(
        FakeHost::new(audit.clone()),
        plans,
        registry.runtime_lease().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap();
    assert!(registry.runtime_lease().is_err());
    for id in ["one", "one", "two", "three"] {
        bundle.prepare(&request(id)).await.unwrap();
    }
    bundle.stop().await.unwrap();
    assert_eq!(
        bundle
            .events()
            .unwrap()
            .iter()
            .filter(|e| e.action == "owned_stop")
            .count(),
        3
    );
    assert!(bundle.prepare(&request("one")).await.is_err());
    let a = audit.lock().unwrap();
    assert_eq!(a.launches, expected);
    assert_eq!(a.maximum_active, 1);
    assert_eq!(
        a.actions,
        vec![
            ("start".into(), "one".into()),
            ("stop".into(), "one".into()),
            ("start".into(), "two".into()),
            ("stop".into(), "two".into()),
            ("start".into(), "three".into()),
            ("stop".into(), "three".into())
        ]
    );
    drop(a);
    drop(bundle);
    assert!(registry.runtime_lease().is_ok());
}
#[tokio::test]
async fn external_runtime_and_material_drift_never_spawn_or_stop_an_external_process() {
    let d = tempfile::tempdir().unwrap();
    let r = Registry::at(&d.path().join("authority")).unwrap();
    let plans = plans(d.path(), 18087, &["one"], 32768);
    let weight = plans[0].model_path.clone();
    let audit = Arc::new(Mutex::new(Audit {
        external: true,
        ..Default::default()
    }));
    let bundle = RuntimeBundle::new(
        FakeHost::new(audit.clone()),
        plans,
        r.runtime_lease().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap();
    assert!(bundle.prepare(&request("one")).await.is_err());
    assert!(audit.lock().unwrap().actions.is_empty());
    audit.lock().unwrap().external = false;
    std::fs::write(weight, b"changed weight").unwrap();
    assert!(bundle.prepare(&request("one")).await.is_err());
    assert!(audit.lock().unwrap().actions.is_empty());
}
#[tokio::test]
async fn cancelled_load_drains_its_owned_child_and_releases_the_generation_permit_without_http() {
    let d = tempfile::tempdir().unwrap();
    let r = Registry::at(&d.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(Audit {
        loading: true,
        ..Default::default()
    }));
    let bundle = Arc::new(
        RuntimeBundle::new(
            FakeHost::new(audit.clone()),
            plans(d.path(), 18087, &["one"], 32768),
            r.runtime_lease().unwrap(),
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let http = ProviderHttp::new(&["http://127.0.0.1:18087".into()], Duration::from_secs(1))
        .unwrap()
        .with_runtime(bundle.clone());
    let scheduler = Scheduler::default();
    scheduler
        .register("http://127.0.0.1:18087", EndpointKind::Local)
        .unwrap();
    let ticket = scheduler
        .submit(
            "http://127.0.0.1:18087",
            ag1_core::OperationId::new("load").unwrap(),
        )
        .unwrap();
    let permit = scheduler.start(&ticket).unwrap().unwrap();
    let (source, mut cancel) = cancellation();
    let cancel_audit = audit.clone();
    let cancelled = tokio::spawn(async move {
        loop {
            if cancel_audit.lock().unwrap().active.is_some() {
                source.cancel();
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
    let result = request_generation(
        &http,
        GenerationRequest {
            request_id: "load".into(),
            tool_names: vec![],
            family: ag1_core::provider::ApiFamily::Completions,
            request: request("one"),
            limits: GenerationLimits {
                max_output_bytes: 1024,
                prefill_ms: 1000,
                idle_ms: 1000,
                overall_ms: 1000,
            },
        },
        permit,
        &mut cancel,
        |_| Ok(()),
    )
    .await;
    assert_eq!(result.unwrap_err().error, DomainError::Cancelled);
    cancelled.await.unwrap();
    bundle.stop().await.unwrap();
    assert!(audit.lock().unwrap().active.is_none());
    assert_eq!(
        bundle.events().unwrap().last().unwrap().action,
        "owned_stop"
    );
    let ticket = scheduler
        .submit(
            "http://127.0.0.1:18087",
            ag1_core::OperationId::new("after").unwrap(),
        )
        .unwrap();
    scheduler
        .start(&ticket)
        .unwrap()
        .unwrap()
        .terminal()
        .unwrap();
}
#[test]
fn resource_observations_only_allow_the_authenticated_owned_pid() {
    use ag1_platform::llama::window::validate;
    assert!(validate(None, &[], &[]).is_ok());
    assert!(validate(None, &[11], &[]).is_err());
    assert!(validate(Some(11), &[11], &[11]).is_ok());
    assert!(validate(Some(11), &[11], &[11, 22]).is_err());
    assert!(validate(Some(11), &[22], &[]).is_err());
}

#[tokio::test]
async fn stopping_a_generation_retains_the_same_loaded_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::at(&dir.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(Audit::default()));
    let bundle = RuntimeBundle::new(
        FakeHost::new(audit.clone()),
        plans(dir.path(), 18087, &["one"], 32768),
        registry.runtime_lease().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap();
    bundle.prepare(&request("one")).await.unwrap();
    assert!(
        bundle
            .cancel_generation("http://127.0.0.1:9999/v1/chat/completions")
            .await
            .is_err()
    );
    assert!(audit.lock().unwrap().active.is_some());
    bundle.cancel_generation(&request("one").url).await.unwrap();
    assert_eq!(audit.lock().unwrap().active.as_deref(), Some("one"));
    assert_eq!(
        bundle.events().unwrap().last().unwrap().action,
        "owned_generation_cancelled"
    );
    bundle.prepare(&request("one")).await.unwrap();
    assert_eq!(
        audit
            .lock()
            .unwrap()
            .actions
            .iter()
            .map(|a| a.0.as_str())
            .collect::<Vec<_>>(),
        vec!["start"]
    );
    bundle.stop().await.unwrap();
}

#[tokio::test]
async fn cancelling_preparation_preserves_the_previously_loaded_model() {
    let directory = tempfile::tempdir().unwrap();
    let registry = Registry::at(&directory.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(Audit::default()));
    let bundle = Arc::new(
        RuntimeBundle::new(
            FakeHost::new(audit.clone()),
            plans(directory.path(), 18087, &["one", "two"], 32768),
            registry.runtime_lease().unwrap(),
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    bundle.load_model("one").await.unwrap();
    for model in ["one", "two"] {
        let calls = {
            let mut state = audit.lock().unwrap();
            state.verification_hold = true;
            state.verification_calls
        };
        let preparing = bundle.clone();
        let task = tokio::spawn(async move { preparing.prepare(&request(model)).await });
        while audit.lock().unwrap().verification_calls == calls {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        audit.lock().unwrap().verification_hold = false;
        // The resources call queues behind the cancelled blocking preparation.
        bundle.resources().await.unwrap();
        assert_eq!(audit.lock().unwrap().active.as_deref(), Some("one"));
        assert_eq!(audit.lock().unwrap().actions.len(), 1);
        assert_eq!(bundle.progress().unwrap()["phase"], "ready");
        assert_eq!(bundle.progress().unwrap()["model"], "one");
    }
    bundle.stop().await.unwrap();
}

#[tokio::test]
async fn cancellation_waits_for_idle_and_keeps_uncertain_runtime_loaded() {
    let directory = tempfile::tempdir().unwrap();
    let registry = Registry::at(&directory.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(Audit::default()));
    let bundle = Arc::new(
        RuntimeBundle::new(
            FakeHost::new(audit.clone()),
            plans(directory.path(), 18087, &["one"], 32768),
            registry.runtime_lease().unwrap(),
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    bundle.prepare(&request("one")).await.unwrap();
    audit.lock().unwrap().generation_busy = true;
    let cancelling = bundle.clone();
    let pending =
        tokio::spawn(async move { cancelling.cancel_generation(&request("one").url).await });
    while audit.lock().unwrap().idle_polls == 0 {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(!pending.is_finished());
    assert_eq!(audit.lock().unwrap().active.as_deref(), Some("one"));
    audit.lock().unwrap().generation_busy = false;
    pending.await.unwrap().unwrap();
    audit.lock().unwrap().idle_observation_error = true;
    assert!(bundle.cancel_generation(&request("one").url).await.is_err());
    assert_eq!(audit.lock().unwrap().actions.len(), 1);
    assert!(registry.runtime_lease().is_err());
    bundle.unload_owned().await.unwrap();
    assert!(audit.lock().unwrap().active.is_none());
    bundle.stop().await.unwrap();
}

#[tokio::test]
async fn cancellation_timeout_never_ejects_the_loaded_model() {
    let directory = tempfile::tempdir().unwrap();
    let registry = Registry::at(&directory.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(Audit::default()));
    let bundle = RuntimeBundle::new(
        FakeHost::new(audit.clone()),
        plans(directory.path(), 18087, &["one"], 32768),
        registry.runtime_lease().unwrap(),
        Duration::from_millis(50),
    )
    .unwrap();
    bundle.prepare(&request("one")).await.unwrap();
    audit.lock().unwrap().generation_busy = true;
    assert_eq!(
        bundle.cancel_generation(&request("one").url).await,
        Err(DomainError::TimedOut)
    );
    assert_eq!(audit.lock().unwrap().active.as_deref(), Some("one"));
    assert_eq!(audit.lock().unwrap().actions.len(), 1);
    bundle.stop().await.unwrap();
}

#[tokio::test]
async fn cancelled_http_stream_drains_and_next_request_reuses_the_loaded_runtime() {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
    };
    fn read_request(stream: &TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        let mut length = 0;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        assert!(length < 8192);
        reader.read_exact(&mut vec![0; length]).unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let origin = format!("http://127.0.0.1:{port}");
    let directory = tempfile::tempdir().unwrap();
    let registry = Registry::at(&directory.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(Audit::default()));
    let server_audit = audit.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        read_request(&stream);
        server_audit.lock().unwrap().generation_busy = true;
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()+10).unwrap();
        // Simulate the engine settling its generation only after client disconnect.
        assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
        server_audit.lock().unwrap().generation_busy = false;
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&stream);
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"next\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let bundle = Arc::new(
        RuntimeBundle::new(
            FakeHost::new(audit.clone()),
            plans(directory.path(), port, &["one"], 32768),
            registry.runtime_lease().unwrap(),
            Duration::from_secs(3),
        )
        .unwrap(),
    );
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1))
        .unwrap()
        .with_runtime(bundle.clone());
    let scheduler = Scheduler::default();
    scheduler.register(&origin, EndpointKind::Local).unwrap();
    for (index, abort) in [true, false].into_iter().enumerate() {
        let ticket = scheduler
            .submit(
                &origin,
                ag1_core::OperationId::new(format!("mock-{index}")).unwrap(),
            )
            .unwrap();
        let permit = scheduler.start(&ticket).unwrap().unwrap();
        let (source, mut cancel) = cancellation();
        let mut request = request("one");
        request.url = format!("{origin}/v1/chat/completions");
        request.max_response_bytes = 8192;
        let result = request_generation(
            &http,
            GenerationRequest {
                request_id: format!("mock-{index}"),
                tool_names: vec![],
                family: ag1_core::provider::ApiFamily::Completions,
                request,
                limits: GenerationLimits {
                    max_output_bytes: 4096,
                    prefill_ms: 3000,
                    idle_ms: 3000,
                    overall_ms: 5000,
                },
            },
            permit,
            &mut cancel,
            |event| {
                if abort && matches!(event, ag1_core::provider::ProviderEvent::Delta { .. }) {
                    source.cancel();
                }
                Ok(())
            },
        )
        .await;
        if abort {
            assert_eq!(result.unwrap_err().error, DomainError::Cancelled);
        } else {
            assert_eq!(result.unwrap().content[0].text, "next");
        }
        assert_eq!(scheduler.status(&origin).unwrap().quarantined, 0);
        assert_eq!(audit.lock().unwrap().active.as_deref(), Some("one"));
        assert_eq!(audit.lock().unwrap().actions.len(), 1);
    }
    server.join().unwrap();
    bundle.stop().await.unwrap();
}

#[tokio::test]
async fn only_requested_model_is_verified_and_missing_or_unknown_selection_never_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::at(&dir.path().join("authority")).unwrap();
    let plans = plans(dir.path(), 18087, &["one", "two", "three"], 32768);
    std::fs::remove_file(&plans[1].model_path).unwrap();
    std::fs::write(&plans[2].model_path, b"changed model bytes").unwrap();
    let audit = Arc::new(Mutex::new(Audit::default()));
    let bundle = RuntimeBundle::new(
        FakeHost::new(audit.clone()),
        plans,
        registry.runtime_lease().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap();
    assert!(audit.lock().unwrap().actions.is_empty());
    bundle.prepare(&request("one")).await.unwrap();
    bundle.prepare(&request("one")).await.unwrap();
    assert_eq!(audit.lock().unwrap().launches.len(), 1);
    assert_eq!(
        bundle.prepare(&request("missing")).await,
        Err(DomainError::PermissionDenied)
    );
    assert_eq!(
        bundle.prepare(&request("two")).await,
        Err(DomainError::NotFound)
    );
    assert_eq!(
        bundle.prepare(&request("three")).await,
        Err(DomainError::Conflict)
    );
    assert_eq!(audit.lock().unwrap().launches.len(), 1);
    bundle.stop().await.unwrap();
}

#[tokio::test]
async fn catalog_wide_capture_verifies_only_selected_runtime_without_model_fallback() {
    use ag1_platform::llama::{ResolveInput, resolve_launch_plan};
    let d = tempfile::tempdir().unwrap();
    let registry = Registry::at(&d.path().join("authority")).unwrap();
    let original = plans(d.path(), 18087, &["35b", "27b", "flash"], 32768);
    let inventory = original
        .iter()
        .flat_map(|p| p.materials.clone())
        .collect::<Vec<_>>();
    let plans = original.into_iter().map(|p| resolve_launch_plan(ResolveInput {
        model_id: p.model_id, model_path: p.model_path,
        profile_bytes: serde_json::to_vec(&serde_json::json!({"serverPath":p.command,"contextWindow":32768,"maxTokens":4096,"nCpuMoe":0,"priority":false})).unwrap(),
        environment: [("AG1_LLAMA_CPP_PORT".into(), "18087".into())].into(),
        context_override: None, forced_moe: None, cached_moe: None, vram_mib: Some(16376),
        physical_cores: 12, logical_cores: 24, discovered_projector: None,
        materials: inventory.clone(),
    }).unwrap()).collect();
    // Mirrors the real capture: every plan contains every model's weights.
    // Unrelated model files must not even be opened while preparing 35b.
    std::fs::remove_file(d.path().join("27b.gguf")).unwrap();
    std::fs::remove_file(d.path().join("flash.gguf")).unwrap();
    let audit = Arc::new(Mutex::new(Audit::default()));
    let bundle = RuntimeBundle::new(
        FakeHost::new(audit.clone()),
        plans,
        registry.runtime_lease().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap();
    bundle.prepare(&request("35b")).await.unwrap();
    bundle.prepare(&request("35b")).await.unwrap();
    assert_eq!(audit.lock().unwrap().launches.len(), 1); // stays warm
    assert_eq!(bundle.progress().unwrap()["phase"], "ready");
    assert_eq!(bundle.progress().unwrap()["model"], "35b");
    assert!(bundle.prepare(&request("flash")).await.is_err());
    assert_eq!(bundle.progress().unwrap()["phase"], "failed");
    assert_eq!(bundle.progress().unwrap()["model"], "flash");
    assert_eq!(audit.lock().unwrap().launches.len(), 1); // no fallback launch
    bundle.stop().await.unwrap();
}

#[tokio::test]
async fn manual_preparation_reuses_resident_model_and_clears_status_on_unload() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::at(&dir.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(Audit::default()));
    let bundle = RuntimeBundle::new(
        FakeHost::new(audit.clone()),
        plans(dir.path(), 18087, &["one", "two"], 32768),
        registry.runtime_lease().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap();
    bundle.load_model("one").await.unwrap();
    assert_eq!(bundle.progress().unwrap()["phase"], "ready");
    bundle.load_model("one").await.unwrap();
    assert_eq!(audit.lock().unwrap().launches.len(), 1);
    bundle.load_model("two").await.unwrap();
    assert_eq!(bundle.progress().unwrap()["model"], "two");
    assert_eq!(audit.lock().unwrap().maximum_active, 1);
    assert!(bundle.load_model("unknown").await.is_err());
    bundle.unload_owned().await.unwrap();
    assert!(bundle.progress().is_none());
    bundle.stop().await.unwrap();
}

#[test]
fn verification_percentage_counts_bytes_and_never_completes_failed_verification() {
    use ag1_platform::llama::{Material, native::MaterialVerifier};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("weight");
    let bytes = vec![42u8; 1024 * 1024];
    std::fs::write(&path, &bytes).unwrap();
    let material = Material {
        path: path.to_string_lossy().into_owned(),
        sha256: ag1_storage::digest(&bytes),
        kind: "weight".into(),
    };
    let mut verifier = MaterialVerifier::default();
    let mut values = vec![];
    verifier
        .verify_progress(std::slice::from_ref(&material), &mut |p| values.push(p))
        .unwrap();
    assert_eq!(values.first(), Some(&0));
    assert_eq!(values.last(), Some(&100));
    assert!(values.len() > 5);
    assert!(values.windows(2).all(|w| w[0] <= w[1]));
    values.clear();
    verifier
        .verify_progress(std::slice::from_ref(&material), &mut |p| values.push(p))
        .unwrap();
    assert_eq!(values.last(), Some(&100));
    std::fs::write(&path, vec![41u8; 1024 * 1024]).unwrap();
    values.clear();
    assert!(
        verifier
            .verify_progress(&[material], &mut |p| values.push(p))
            .is_err()
    );
    assert!(!values.contains(&100));
}
