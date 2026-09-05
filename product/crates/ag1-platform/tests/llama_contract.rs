use ag1_core::DomainError;
use ag1_platform::llama::lifecycle::*;
use ag1_platform::llama::*;
use serde_json::{Value, json};
fn input(profile: Value) -> ResolveInput {
    ResolveInput {
        model_id: "Qwen3-A3B".into(),
        model_path: "/fixture/model.gguf".into(),
        profile_bytes: serde_json::to_vec(&profile).unwrap(),
        environment: [("AG1_LLAMA_SERVER".into(), "/fixture/server".into())].into(),
        context_override: None,
        forced_moe: None,
        cached_moe: None,
        vram_mib: Some(24576),
        physical_cores: 12,
        logical_cores: 24,
        discovered_projector: None,
        materials: vec![],
    }
}
fn flag(plan: &LaunchPlan, name: &str) -> Option<String> {
    plan.args
        .windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].clone())
}
#[test]
fn all_nine_pinned_ts_launch_captures_match_order_and_effective_environment() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../../fixtures/llama/launch-plans.json")).unwrap();
    let captures = fixtures["captures"].as_array().unwrap();
    assert_eq!(captures.len(), 9);
    for capture in captures {
        let mut i = input(capture["inputs"]["profile"].clone());
        i.model_id = capture["model"].as_str().unwrap().into();
        i.model_path = format!("<fixture>/{}.gguf", i.model_id);
        i.context_override = capture["inputs"]["contextOverride"].as_u64();
        i.environment = serde_json::from_value(capture["inputs"]["environment"].clone()).unwrap();
        let p = resolve_launch_plan(i).unwrap();
        assert_eq!(
            p.command, capture["observed"]["command"],
            "{}",
            capture["id"]
        );
        assert_eq!(
            json!(p.args),
            capture["observed"]["args"],
            "{}",
            capture["id"]
        );
        assert_eq!(
            json!(p.environment),
            capture["observed"]["environment"],
            "{}",
            capture["id"]
        );
        let model = &capture["observed"]["providerConfig"]["models"][0];
        assert_eq!(
            p.requested_context,
            model["contextWindow"].as_u64().unwrap()
        );
        assert_eq!(p.max_tokens, model["maxTokens"].as_u64().unwrap());
    }
}
#[test]
fn absent_false_zero_tiers_and_projector_speculation_stay_distinct() {
    let mut i = input(
        json!({"priority":false,"nCpuMoe":0,"kvOffload":false,"kvUnified":false,"flashAttention":false,"slotSimilarityFlag":"none","mmprojPath":"/fixture/projector.gguf","contextBatchTiers":[{"maxContextWindow":90000,"ubatchSize":1024}]}),
    );
    let p = resolve_launch_plan(i).unwrap();
    assert!(!p.args.contains(&"--prio".into()));
    assert!(!p.args.contains(&"--slot-cache-key-similarity".into()));
    assert!(!p.args.contains(&"--kv-unified".into()));
    assert!(p.args.contains(&"--no-kv-offload".into()));
    assert_eq!(flag(&p, "--n-cpu-moe").as_deref(), Some("0"));
    assert_eq!(flag(&p, "--flash-attn").as_deref(), Some("off"));
    assert_eq!(flag(&p, "--ubatch-size").as_deref(), Some("1024"));
    assert_eq!(
        flag(&p, "--mmproj").as_deref(),
        Some("/fixture/projector.gguf")
    );
    i = input(
        json!({"specType":"draft-mtp","mmprojPath":"/fixture/projector.gguf","extraArgs":["--spec-draft-n-max","3"]}),
    );
    let p = resolve_launch_plan(i).unwrap();
    assert_eq!(flag(&p, "--prio").as_deref(), Some("2"));
    assert_eq!(
        flag(&p, "--slot-cache-key-similarity").as_deref(),
        Some("0")
    );
    assert!(p.projector.is_none());
    assert_eq!(&p.args[p.args.len() - 2..], ["--spec-draft-n-max", "3"]);
}
#[test]
fn explicit_context_and_environment_moe_precede_profile_vram_and_hardware() {
    for (env, forced, profile, cached, expected) in [
        (Some(0), Some(3), Some(5), Some(7), 0),
        (None, Some(3), Some(5), Some(7), 3),
        (None, None, Some(5), Some(7), 5),
        (None, None, None, Some(7), 7),
        (None, None, None, None, 12),
    ] {
        let mut i = input(json!({"nCpuMoe":profile,"contextWindow":65536}));
        i.context_override = Some(90001);
        i.forced_moe = forced;
        i.cached_moe = cached;
        if let Some(n) = env {
            i.environment
                .insert("AG1_LLAMA_N_CPU_MOE".into(), n.to_string());
        }
        i.environment.insert("AG1_LLAMA_CTX".into(), "4096".into());
        let p = resolve_launch_plan(i).unwrap();
        assert_eq!(p.requested_context, 90001);
        assert_eq!(flag(&p, "--n-cpu-moe"), Some(expected.to_string()));
    }
}
#[test]
fn profile_library_is_not_inferred_and_profile_environment_precedes_inheritance() {
    let mut i = input(
        json!({"serverPath":"/fixture/model-specific","environment":{"LD_LIBRARY_PATH":"/profile/inherited"}}),
    );
    i.environment
        .insert("AG1_LLAMA_LIB_PATH".into(), "/environment/lib".into());
    let p = resolve_launch_plan(i).unwrap();
    assert_eq!(p.environment["LD_LIBRARY_PATH"], "/profile/inherited");
    let i = input(
        json!({"serverPath":"/fixture/model-specific","libPath":"/selected/lib","environment":{"LD_LIBRARY_PATH":"/profile/inherited"}}),
    );
    let p = resolve_launch_plan(i).unwrap();
    assert_eq!(
        p.environment["LD_LIBRARY_PATH"],
        "/selected/lib:/profile/inherited"
    );
}

struct FakeHost {
    observation: Observation,
    material_ok: bool,
    alive: bool,
    same_process: bool,
    spawn_fails: bool,
    spawns: usize,
    stops: usize,
}
impl Default for FakeHost {
    fn default() -> Self {
        Self {
            observation: Observation::Vacant,
            material_ok: true,
            alive: true,
            same_process: true,
            spawn_fails: false,
            spawns: 0,
            stops: 0,
        }
    }
}
struct FakeOwned;
impl RuntimeHost for FakeHost {
    type Handle = FakeOwned;
    fn verify_materials(&mut self, _: &[Material]) -> Result<(), DomainError> {
        if self.material_ok {
            Ok(())
        } else {
            Err(DomainError::Conflict)
        }
    }
    fn observe(&mut self, _: &LaunchPlan) -> Result<Observation, DomainError> {
        Ok(self.observation.clone())
    }
    fn spawn(&mut self, _: &LaunchPlan) -> Result<FakeOwned, DomainError> {
        self.spawns += 1;
        if self.spawn_fails {
            Err(DomainError::Unavailable("fixture load failure".into()))
        } else {
            Ok(FakeOwned)
        }
    }
    fn owned_alive(&mut self, _: &FakeOwned) -> Result<bool, DomainError> {
        if self.same_process {
            Ok(self.alive)
        } else {
            Err(DomainError::Conflict)
        }
    }
    fn stop(&mut self, _: &mut FakeOwned) -> Result<(), DomainError> {
        if !self.same_process {
            return Err(DomainError::Conflict);
        }
        self.stops += 1;
        self.alive = false;
        Ok(())
    }
}
fn locked_plan() -> LaunchPlan {
    let mut i = input(json!({}));
    i.materials = vec![
        Material {
            path: "/fixture/server".into(),
            sha256: "a".repeat(64),
            kind: "executable".into(),
        },
        Material {
            path: i.model_path.clone(),
            sha256: "b".repeat(64),
            kind: "weight".into(),
        },
    ];
    resolve_launch_plan(i).unwrap()
}
fn healthy(plan: &LaunchPlan) -> Observation {
    Observation::Healthy(ModelIdentity {
        path: Some(plan.model_path.clone()),
        alias: Some(plan.model_id.clone()),
        model: None,
        context_window: Some(plan.requested_context),
        vision: false,
    })
}
#[test]
fn external_health_is_never_process_ownership_or_permission_to_evict() {
    let p = locked_plan();
    for observation in [Observation::Occupied, healthy(&p)] {
        let mut controller = RuntimeController::new(FakeHost {
            observation,
            ..Default::default()
        });
        controller.observe_external_runtime(&p).unwrap();
        assert!(!controller.owns_process());
        assert_eq!(
            controller.start_owned_runtime(&p),
            Err(DomainError::Conflict)
        );
        controller.stop_owned_runtime().unwrap();
        assert_eq!((controller.host().spawns, controller.host().stops), (0, 0));
    }
}
#[test]
fn missing_mismatched_material_and_launch_drift_fail_before_spawn() {
    let mut p = locked_plan();
    let mut controller = RuntimeController::new(FakeHost {
        material_ok: false,
        ..Default::default()
    });
    assert_eq!(
        controller.start_owned_runtime(&p),
        Err(DomainError::Conflict)
    );
    assert_eq!(controller.host().spawns, 0);
    controller.host_mut().material_ok = true;
    p.args.push("--changed".into());
    assert_eq!(
        controller.start_owned_runtime(&p),
        Err(DomainError::Conflict)
    );
    assert_eq!(controller.host().spawns, 0);
    p = resolve_launch_plan(input(json!({}))).unwrap();
    assert!(matches!(
        controller.start_owned_runtime(&p),
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(controller.host().spawns, 0);
}
#[test]
fn owned_loading_readiness_quit_restart_and_stale_process_are_separate() {
    let p = locked_plan();
    let mut controller = RuntimeController::new(FakeHost::default());
    controller.start_owned_runtime(&p).unwrap();
    assert_eq!(controller.state(), &RuntimeState::Loading);
    for _ in 0..20 {
        assert!(!controller.poll_ready(&p).unwrap());
    }
    controller.host_mut().observation = healthy(&p);
    assert!(controller.poll_ready(&p).unwrap());
    controller.host_mut().same_process = false;
    assert_eq!(controller.stop_owned_runtime(), Err(DomainError::Conflict));
    assert_eq!(controller.host().stops, 0);
    assert!(controller.owns_process());
    controller.host_mut().same_process = true;
    controller.stop_owned_runtime().unwrap();
    controller.stop_owned_runtime().unwrap();
    assert_eq!(controller.host().stops, 1);
    controller.host_mut().observation = Observation::Vacant;
    controller.host_mut().alive = true;
    controller.start_owned_runtime(&p).unwrap();
    assert_eq!(controller.host().spawns, 2);
    controller.host_mut().alive = false;
    assert!(matches!(
        controller.poll_ready(&p),
        Err(DomainError::Unavailable(_))
    ));
    assert!(!controller.owns_process());
}
#[test]
fn wrong_model_context_load_failure_and_deadline_are_visible() {
    let p = locked_plan();
    for identity in [
        ModelIdentity {
            path: Some("/wrong".into()),
            alias: None,
            model: None,
            context_window: Some(p.requested_context),
            vision: false,
        },
        ModelIdentity {
            path: Some(p.model_path.clone()),
            alias: None,
            model: None,
            context_window: Some(4096),
            vision: false,
        },
    ] {
        let mut controller = RuntimeController::new(FakeHost::default());
        controller.start_owned_runtime(&p).unwrap();
        controller.host_mut().observation = Observation::Healthy(identity);
        assert_eq!(controller.poll_ready(&p), Err(DomainError::Conflict));
        controller.stop_owned_runtime().unwrap();
    }
    let mut controller = RuntimeController::new(FakeHost {
        spawn_fails: true,
        ..Default::default()
    });
    assert!(controller.start_owned_runtime(&p).is_err());
    assert!(!controller.owns_process());
    controller.host_mut().spawn_fails = false;
    controller.start_owned_runtime(&p).unwrap();
    assert_eq!(controller.timeout_loading(), Err(DomainError::TimedOut));
    assert!(!controller.owns_process());
    assert_eq!(controller.host().stops, 1);
}

#[cfg(target_os = "linux")]
#[test]
fn native_verifier_rejects_missing_library_same_size_tamper_and_changed_executable() {
    use ag1_platform::llama::native::MaterialVerifier;
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.so");
    std::fs::write(&path, b"pinned").unwrap();
    let material = Material {
        path: path.display().to_string(),
        sha256: format!("{:x}", Sha256::digest(b"pinned")),
        kind: "library".into(),
    };
    let mut verifier = MaterialVerifier::default();
    verifier.verify(std::slice::from_ref(&material)).unwrap();
    verifier.verify(std::slice::from_ref(&material)).unwrap();
    std::fs::write(&path, b"tamper").unwrap();
    assert_eq!(
        verifier.verify(std::slice::from_ref(&material)),
        Err(DomainError::Conflict)
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        verifier.verify(std::slice::from_ref(&material)),
        Err(DomainError::NotFound)
    );
    std::fs::write(&path, b"pinned").unwrap();
    let mut executable = material;
    executable.kind = "executable".into();
    verifier.verify(std::slice::from_ref(&executable)).unwrap();
    std::fs::write(&path, b"changed executable").unwrap();
    assert_eq!(verifier.verify(&[executable]), Err(DomainError::Conflict));
}
#[cfg(target_os = "linux")]
#[test]
fn native_runtime_reuses_supervisor_and_exact_argv_without_running_llama() {
    use ag1_platform::llama::native::NativeRuntimeHost;
    use sha2::{Digest, Sha256};
    use std::{
        os::unix::fs::PermissionsExt,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let record = dir.path().join("fake-launch.json");
    let weight = dir.path().join("model.gguf");
    std::fs::write(&weight, b"fake weight").unwrap();
    let command = env!("CARGO_BIN_EXE_ag1-platform-fixture");
    let mut i = input(
        json!({"serverPath":command,"priority":false,"slotSimilarityFlag":"none","environment":{"AG1_MODEL_FIXTURE_RECORD":record},"extraArgs":["--fixture-extra","preserved"]}),
    );
    i.model_path = weight.display().to_string();
    i.materials = vec![
        Material {
            path: command.into(),
            sha256: format!("{:x}", Sha256::digest(std::fs::read(command).unwrap())),
            kind: "executable".into(),
        },
        Material {
            path: i.model_path.clone(),
            sha256: format!("{:x}", Sha256::digest(b"fake weight")),
            kind: "weight".into(),
        },
    ];
    let plan = resolve_launch_plan(i).unwrap();
    let observation = Arc::new(Mutex::new(Observation::Vacant));
    let cloned = observation.clone();
    let host = NativeRuntimeHost::new(
        std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
        dir.path(),
        move |_: &LaunchPlan| Ok(cloned.lock().unwrap().clone()),
    )
    .unwrap();
    let mut controller = RuntimeController::new(host);
    controller.start_owned_runtime(&plan).unwrap();
    let start = Instant::now();
    while !record.exists() && start.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let captured: Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    assert_eq!(captured["args"], json!(plan.args));
    *observation.lock().unwrap() = healthy(&plan);
    assert!(controller.poll_ready(&plan).unwrap());
    let pid = captured["pid"].as_u64().unwrap();
    controller.stop_owned_runtime().unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert!(!controller.owns_process());
}

#[test]
fn discovery_split_sets_projectors_and_explicit_tuning_match_reference() {
    use ag1_platform::llama::discovery::*;
    use std::{fs, path::Path};
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/providers/discovery.json"
    ))
    .unwrap();
    for case in fixture["pure"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        assert_eq!(usable_gguf(name), case["usable"], "{name}");
        assert_eq!(
            serde_json::to_value(split_gguf(name)).unwrap(),
            case["split"],
            "{name}"
        );
        assert_eq!(family_key(Path::new(name)), case["family"], "{name}");
    }
    let dir = tempfile::tempdir().unwrap();
    for (name, size) in fixture["discovery"]["files"].as_object().unwrap() {
        let p = dir.path().join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0; size.as_u64().unwrap() as usize]).unwrap();
    }
    let models = discover_models(&[dir.path().into()]).unwrap();
    let got:Vec<_>=models.iter().map(|m|json!({"id":m.id,"path":m.path.strip_prefix(dir.path()).unwrap(),"sizeBytes":m.size_bytes})).collect();
    assert_eq!(json!(got), fixture["discovery"]["models"]);
    let model = dir.path().join("Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf");
    let chosen = discover_projector(&model, &[dir.path().into()], None)
        .unwrap()
        .unwrap();
    assert_eq!(
        chosen.strip_prefix(dir.path()).unwrap().to_str().unwrap(),
        fixture["discovery"]["projector"]
    );
    for disabled in ["0", "off", "false", "none"] {
        assert!(
            discover_projector(&model, &[dir.path().into()], Some(disabled))
                .unwrap()
                .is_none()
        );
    }
    for case in fixture["tuning"].as_array().unwrap() {
        let proposal = ag1_core::tuning::propose(ag1_core::tuning::ExplicitTuningRequest {
            version: 1,
            operation: ag1_core::OperationId::new("explicit-fixture-proposal").unwrap(),
            profile_sha256: "a".repeat(64),
            current_n: case["currentN"].as_u64().unwrap() as u32,
            headroom_mib: case["headroomMiB"].as_f64().unwrap(),
            estimated_mib_per_layer: None,
        })
        .unwrap();
        assert_eq!(proposal.proposed_n, case["expected"]);
    }
    // A missing shard disappears from the picker, without changing any other files.
    fs::remove_file(dir.path().join("Flash-00002-of-00003.gguf")).unwrap();
    assert!(
        !discover_models(&[dir.path().into()])
            .unwrap()
            .iter()
            .any(|m| m.id == "Flash")
    );
}

#[test]
fn authenticated_runtime_handoff_rechecks_identity_and_preserves_cleanup_on_failure() {
    let p = locked_plan();
    let mut c = RuntimeController::new(FakeHost::default());
    c.start_owned_runtime(&p).unwrap();
    c.host_mut().observation = healthy(&p);
    let mut c = match c.handoff().restore(&p) {
        Ok(c) => c,
        Err(_) => panic!("verified handle failed"),
    };
    assert_eq!(c.host().spawns, 1);
    assert!(c.owns_process());
    c.host_mut().same_process = false;
    let transfer = c.handoff();
    match transfer.restore(&p) {
        Ok(_) => panic!("stale process restored"),
        Err((error, _)) => assert_eq!(error, DomainError::Conflict),
    }
    let mut c = RuntimeController::new(FakeHost::default());
    c.observe_external_runtime(&p).unwrap();
    assert!(c.handoff().restore(&p).is_err());
}
