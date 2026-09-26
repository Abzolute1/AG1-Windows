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
async fn stopping_a_generation_proves_owned_exit_without_closing_the_bundle() {
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
    assert!(audit.lock().unwrap().active.is_none());
    bundle.prepare(&request("one")).await.unwrap();
    assert_eq!(
        audit
            .lock()
            .unwrap()
            .actions
            .iter()
            .map(|a| a.0.as_str())
            .collect::<Vec<_>>(),
        vec!["start", "stop", "start"]
    );
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
