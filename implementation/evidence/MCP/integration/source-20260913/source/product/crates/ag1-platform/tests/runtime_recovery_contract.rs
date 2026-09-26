//! Real P14 crash regression: only authenticated owned exits may be restarted.
#[path = "../../../../fixtures/runtime/custody.rs"]
mod fixture;
use ag1_core::DomainError;
use ag1_platform::{
    cutover::Registry,
    llama::{
        LaunchPlan, Material,
        custody::RuntimeBundle,
        lifecycle::{Observation, RuntimeHost},
    },
    provider::http::{GenerationRuntime, HostHeaders, HttpMethod, HttpRequest},
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn request() -> HttpRequest {
    HttpRequest {
        url: "http://127.0.0.1:18087/v1/chat/completions".into(),
        method: HttpMethod::Post,
        headers: HostHeaders::default(),
        body: br#"{"model":"one","messages":[]}"#.to_vec(),
        max_response_bytes: 1024,
    }
}
#[derive(Default)]
struct Fault {
    exited: bool,
    stop_fails: bool,
}
struct CrashHost {
    inner: fixture::FakeHost,
    fault: Arc<Mutex<Fault>>,
}
impl RuntimeHost for CrashHost {
    type Handle = String;
    fn verify_materials(&mut self, m: &[Material]) -> Result<(), DomainError> {
        self.inner.verify_materials(m)
    }
    fn observe(&mut self, p: &LaunchPlan) -> Result<Observation, DomainError> {
        self.inner.observe(p)
    }
    fn spawn(&mut self, p: &LaunchPlan) -> Result<String, DomainError> {
        let handle = self.inner.spawn(p)?;
        self.fault.lock().unwrap().exited = false;
        Ok(handle)
    }
    fn owned_alive(&mut self, h: &String) -> Result<bool, DomainError> {
        Ok(!self.fault.lock().unwrap().exited && self.inner.owned_alive(h)?)
    }
    fn stop(&mut self, h: &mut String) -> Result<(), DomainError> {
        if self.fault.lock().unwrap().stop_fails {
            return Err(DomainError::TimedOut);
        }
        self.inner.stop(h)
    }
}
#[tokio::test]
async fn next_request_restarts_a_verified_exited_model_without_releasing_custody() {
    let d = tempfile::tempdir().unwrap();
    let registry = Registry::at(&d.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(fixture::Audit::default()));
    let fault = Arc::new(Mutex::new(Fault::default()));
    let bundle = RuntimeBundle::new(
        CrashHost {
            inner: fixture::FakeHost::new(audit.clone()),
            fault: fault.clone(),
        },
        fixture::plans(d.path(), 18087, &["one"], 32768),
        registry.runtime_lease().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap();
    bundle.prepare(&request()).await.unwrap();
    fault.lock().unwrap().exited = true;
    bundle.prepare(&request()).await.unwrap();
    assert!(registry.runtime_lease().is_err());
    assert_eq!(audit.lock().unwrap().launches.len(), 2);
    assert_eq!(audit.lock().unwrap().maximum_active, 1);
    assert_eq!(
        bundle
            .events()
            .unwrap()
            .iter()
            .filter(|e| e.action == "owned_exit")
            .count(),
        1
    );
    bundle.stop().await.unwrap();
    assert!(audit.lock().unwrap().active.is_none());
}
#[tokio::test]
async fn unresolved_owned_settlement_keeps_the_handle_and_blocks_replacement() {
    let d = tempfile::tempdir().unwrap();
    let registry = Registry::at(&d.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(fixture::Audit::default()));
    let fault = Arc::new(Mutex::new(Fault::default()));
    let bundle = RuntimeBundle::new(
        CrashHost {
            inner: fixture::FakeHost::new(audit.clone()),
            fault: fault.clone(),
        },
        fixture::plans(d.path(), 18087, &["one"], 32768),
        registry.runtime_lease().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap();
    bundle.prepare(&request()).await.unwrap();
    *fault.lock().unwrap() = Fault {
        exited: true,
        stop_fails: true,
    };
    assert_eq!(bundle.prepare(&request()).await, Err(DomainError::TimedOut));
    assert_eq!(audit.lock().unwrap().launches.len(), 1);
    assert!(registry.runtime_lease().is_err());
    fault.lock().unwrap().stop_fails = false;
    bundle.prepare(&request()).await.unwrap();
    assert_eq!(audit.lock().unwrap().launches.len(), 2);
    bundle.stop().await.unwrap();
}
#[tokio::test]
async fn an_external_replacement_after_owned_exit_is_never_adopted_or_evicted() {
    let d = tempfile::tempdir().unwrap();
    let registry = Registry::at(&d.path().join("authority")).unwrap();
    let audit = Arc::new(Mutex::new(fixture::Audit::default()));
    let fault = Arc::new(Mutex::new(Fault::default()));
    let bundle = RuntimeBundle::new(
        CrashHost {
            inner: fixture::FakeHost::new(audit.clone()),
            fault: fault.clone(),
        },
        fixture::plans(d.path(), 18087, &["one"], 32768),
        registry.runtime_lease().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap();
    bundle.prepare(&request()).await.unwrap();
    fault.lock().unwrap().exited = true;
    audit.lock().unwrap().external = true;
    assert_eq!(bundle.prepare(&request()).await, Err(DomainError::Conflict));
    assert_eq!(audit.lock().unwrap().launches.len(), 1);
    assert!(audit.lock().unwrap().external);
    bundle.stop().await.unwrap();
    assert!(audit.lock().unwrap().external);
}

#[cfg(target_os = "linux")]
mod native {
    use super::*;
    use ag1_platform::{
        llama::{
            lifecycle::ModelIdentity,
            native::{NativeOwnedRuntime, NativeRuntimeHost},
        },
        process::{ProcessIdentity, observe},
    };
    use std::{
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::fs::PermissionsExt,
        },
        path::Path,
    };
    struct Recorded<H: RuntimeHost<Handle = NativeOwnedRuntime>> {
        inner: H,
        identities: Arc<Mutex<Vec<ProcessIdentity>>>,
    }
    impl<H: RuntimeHost<Handle = NativeOwnedRuntime>> RuntimeHost for Recorded<H> {
        type Handle = NativeOwnedRuntime;
        fn verify_materials(&mut self, m: &[Material]) -> Result<(), DomainError> {
            self.inner.verify_materials(m)
        }
        fn observe(&mut self, p: &LaunchPlan) -> Result<Observation, DomainError> {
            self.inner.observe(p)
        }
        fn spawn(&mut self, p: &LaunchPlan) -> Result<Self::Handle, DomainError> {
            let h = self.inner.spawn(p)?;
            self.identities
                .lock()
                .unwrap()
                .push(h.target_identity().clone());
            Ok(h)
        }
        fn owned_alive(&mut self, h: &Self::Handle) -> Result<bool, DomainError> {
            self.inner.owned_alive(h)
        }
        fn stop(&mut self, h: &mut Self::Handle) -> Result<(), DomainError> {
            self.inner.stop(h)
        }
    }
    #[tokio::test]
    async fn actual_supervised_fixture_exit_is_reaped_before_the_next_owned_start() {
        let d = tempfile::tempdir().unwrap();
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let registry = Registry::at(&d.path().join("authority")).unwrap();
        let command = env!("CARGO_BIN_EXE_ag1-platform-fixture");
        let weight = d.path().join("model.gguf");
        std::fs::write(&weight, b"fixture weight").unwrap();
        let plan=ag1_platform::llama::resolve_launch_plan(ag1_platform::llama::ResolveInput {
            model_id:"one".into(),model_path:weight.display().to_string(),
            profile_bytes:serde_json::to_vec(&serde_json::json!({"serverPath":command,"contextWindow":32768,"maxTokens":4096,"nCpuMoe":0,"priority":false,"environment":{"AG1_MODEL_FIXTURE_RECORD":d.path().join("launch.json")}})).unwrap(),
            environment:[("AG1_LLAMA_CPP_PORT".into(),"18087".into())].into(),context_override:None,forced_moe:None,cached_moe:None,vram_mib:Some(16376),physical_cores:12,logical_cores:24,discovered_projector:None,
            materials:vec![Material{path:command.into(),sha256:ag1_storage::digest(&std::fs::read(command).unwrap()),kind:"executable".into()},Material{path:weight.display().to_string(),sha256:ag1_storage::digest(b"fixture weight"),kind:"weight".into()}],
        }).unwrap();
        let identities = Arc::new(Mutex::new(Vec::<ProcessIdentity>::new()));
        let observed = identities.clone();
        let host = NativeRuntimeHost::new(
            Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
            d.path(),
            move |p: &LaunchPlan| {
                let rows = observed.lock().unwrap();
                let Some(identity) = rows.last() else {
                    return Ok(Observation::Vacant);
                };
                match observe(identity.pid) {
                    Ok(current) if current == *identity => {
                        Ok(Observation::Healthy(ModelIdentity {
                            path: Some(p.model_path.clone()),
                            alias: Some(p.model_id.clone()),
                            model: None,
                            context_window: Some(p.requested_context),
                            vision: false,
                        }))
                    }
                    Err(DomainError::NotFound) => Ok(Observation::Vacant),
                    _ => Err(DomainError::Conflict),
                }
            },
        )
        .unwrap();
        let bundle = RuntimeBundle::new(
            Recorded {
                inner: host,
                identities: identities.clone(),
            },
            vec![plan],
            registry.runtime_lease().unwrap(),
            Duration::from_secs(10),
        )
        .unwrap();
        bundle.prepare(&request()).await.unwrap();
        let identity = identities.lock().unwrap()[0].clone();
        // This identity comes from the non-serializable handle returned by our
        // own NativeRuntimeHost spawn. Hold a pidfd and recheck it before signal.
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, identity.pid, 0) } as i32;
        assert!(raw >= 0);
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        assert_eq!(observe(identity.pid).unwrap(), identity);
        assert_eq!(
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd.as_raw_fd(),
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            },
            0
        );
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut poll, 1, 5000) }, 1);
        bundle.prepare(&request()).await.unwrap();
        assert_eq!(identities.lock().unwrap().len(), 2);
        {
            let captured = identities.lock().unwrap();
            assert_ne!(captured[0], captured[1]);
        }
        bundle.stop().await.unwrap();
        for identity in identities.lock().unwrap().iter() {
            assert_eq!(observe(identity.pid), Err(DomainError::NotFound));
        }
    }
}
