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
