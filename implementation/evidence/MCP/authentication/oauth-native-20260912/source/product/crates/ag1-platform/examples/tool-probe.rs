//! P09 synthetic native measurements in a newly-created disposable workspace.
//! Timings separate pure preparation, durable dispatch, I/O, spool and optional helper.
use ag1_core::{
    Clock, SessionId,
    context::{ContextArtifactPort, features::Features},
    resources::{
        catalog::{ExtensionResource, Trust},
        prompts::SourceInfo,
        trust,
    },
    tools::{
        approval::{AuthorizedCall, Authorizer, ExecutionFence, Preflight, PreflightContext},
        executor::{self, AdapterOutcome, Effect, Execution, NoHooks},
        policy::Policy,
        registry::{Origin, Registry, Scope, ValidatedCall},
        result::ResultBudget,
    },
};
use ag1_platform::{
    extension_host::{self, DenyBridge, ExtensionService},
    file_tools::{FileExecution, FileService},
    process::ProcessSupervisor,
    resource_fs::{ResourceFs, ScanControl},
    shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
    workspace::WorkspaceService,
};
use ag1_storage::{Store, context::ContextStore, digest};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::Path,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
struct FixtureClock;
impl Clock for FixtureClock {
    fn now_ms(&self) -> u64 {
        1000
    }
}
fn authorize(call: ValidatedCall, scope: &Scope, target: &str) -> AuthorizedCall {
    let policy: Policy =
        serde_json::from_value(json!({"mode":"disabled","cwd":"/fixture"})).unwrap();
    let Preflight::Ready(ticket) = Authorizer::new("measurement")
        .unwrap()
        .preflight(
            call,
            PreflightContext {
                scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: target,
                now: 1000,
                ttl: 60000,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!("fixture admission")
    };
    ticket
}
fn control(cancel: &AtomicBool) -> ScanControl<'_> {
    ScanControl {
        cancel,
        deadline: Instant::now() + Duration::from_secs(30),
    }
}
fn context<'a>(store: &'a Store, operation: &str) -> ContextStore<'a> {
    ContextStore::new(
        store,
        SessionId::new("fixture").unwrap(),
        "fixture",
        operation,
        "2026-09-05T00:00:00.000Z",
        Features::default(),
    )
    .unwrap()
}
fn execution<'a>(registry: &'a Registry, scope: &'a Scope, target: &'a str) -> Execution<'a> {
    Execution {
        registry,
        fence: ExecutionFence {
            scope,
            policy_revision: 1,
            target_revision: target,
            now: 1000,
        },
        budget: ResultBudget::default(),
        clock: &FixtureClock,
    }
}
fn timed<T>(run: impl FnOnce() -> T) -> (T, u64) {
    let start = Instant::now();
    let result = run();
    (result, start.elapsed().as_nanos().try_into().unwrap())
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        3,
        "tool-probe <existing-fixture-parent> <new-report>"
    );
    let dir = tempfile::Builder::new()
        .prefix("p09-tools-")
        .tempdir_in(&args[1])
        .unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let spools = dir.path().join("spools");
    fs::create_dir(&spools).unwrap();
    let service = WorkspaceService::new().unwrap();
    let workspace = service.register(&root).unwrap();
    let files = FileService::new(workspace.clone());
    let store = Store::open(&dir.path().join("store")).unwrap();
    store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":root,"timestamp":"2026-09-05T00:00:00.000Z"}).to_string().as_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[7; 32])
        .unwrap();
    let scope = Scope {
        workspace: workspace.id().as_str().into(),
        session: "fixture".into(),
        epoch: 1,
        revision: 0,
    };
    let mut registry = Registry::builtins().unwrap();
    registry.register(json!({"name":"noop","description":"Owned no-op fixture","parameters":{"type":"object","properties":{},"additionalProperties":false}}),Origin::TrustedHost).unwrap();
    let names = ["noop", "read", "append", "bash"].map(str::to_owned);
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    let cancel = AtomicBool::new(false);
    let mut prepare = vec![];
    let mut durable = vec![];
    for sample in 0..25 {
        let (_, ns) = timed(|| {
            for _ in 0..1000 {
                std::hint::black_box(authorize(
                    registry
                        .prepare("noop-prepare", "call", "noop", &json!({}), true)
                        .unwrap(),
                    &scope,
                    "noop",
                ));
            }
        });
        prepare.push(ns);
        let operation = format!("noop-{sample}");
        let call = registry
            .prepare(&operation, "call", "noop", &json!({}), true)
            .unwrap();
        let ticket = authorize(call, &scope, "noop");
        let (out, ns) = timed(|| {
            executor::execute(
                &store,
                &mut context(&store, &operation),
                ticket,
                execution(&registry, &scope, "noop"),
                &mut NoHooks,
                |ticket| {
                    ticket.consume(&registry, execution(&registry, &scope, "noop").fence)?;
                    Ok(AdapterOutcome {
                        result: json!({"content":[{"type":"text","text":"ok"}]}),
                        effect: Effect::None,
                        artifact_page: false,
                    })
                },
                |_| {},
            )
            .unwrap()
        });
        assert_eq!(out.result["content"][0]["text"], "ok");
        durable.push(ns);
    }
    let text = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ!\n".repeat(512);
    fs::write(root.join("input"), &text).unwrap();
    fs::write(root.join("append"), "").unwrap();
    let mut sequential = vec![];
    let mut parallel = vec![];
    let mut mutations = vec![];
    let mut read_hash = None;
    let mut read_peak = 0;
    let mut write_peak = 0;
    for sample in 0..25 {
        for (concurrent, mutation) in [(false, false), (true, false), (true, true)] {
            let active = AtomicUsize::new(0);
            let peak = AtomicUsize::new(0);
            let barrier = Barrier::new(8);
            let work = |index| {
                let name = if mutation { "append" } else { "read" };
                let operation = format!("io-{sample}-{name}-{index}");
                let args = if mutation {
                    json!({"path":"append","content":"."})
                } else {
                    json!({"path":"input"})
                };
                let call = registry
                    .prepare(&operation, "call", name, &args, true)
                    .unwrap();
                let plan = files
                    .prepare_queued(&call, &scope, &cancel, control(&cancel).deadline)
                    .unwrap();
                let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                if concurrent && !mutation {
                    barrier.wait();
                }
                let ticket = authorize(call, &scope, plan.target_revision());
                // Count the period owning the queued proposal; the native write
                // permit is released by execute before the next proposal returns.
                active.fetch_sub(1, Ordering::SeqCst);
                let out = files
                    .execute(
                        plan,
                        ticket,
                        FileExecution {
                            registry: &registry,
                            scope: &scope,
                            policy_revision: 1,
                            now: || 1000,
                            cancel: &cancel,
                            deadline: control(&cancel).deadline,
                        },
                    )
                    .unwrap();
                digest(out.result.to_string().as_bytes())
            };
            let (hashes, ns) = timed(|| {
                if concurrent {
                    std::thread::scope(|threads| {
                        let handles: Vec<_> = (0..8)
                            .map(|index| threads.spawn(move || work(index)))
                            .collect();
                        handles
                            .into_iter()
                            .map(|handle| handle.join().unwrap())
                            .collect::<Vec<_>>()
                    })
                } else {
                    (0..8).map(work).collect::<Vec<_>>()
                }
            });
            if !mutation {
                for hash in hashes {
                    if let Some(expected) = &read_hash {
                        assert_eq!(&hash, expected);
                    } else {
                        read_hash = Some(hash);
                    }
                }
                if concurrent {
                    parallel.push(ns);
                    read_peak = read_peak.max(peak.load(Ordering::SeqCst));
                } else {
                    sequential.push(ns);
                }
            } else {
                mutations.push(ns);
                write_peak = write_peak.max(peak.load(Ordering::SeqCst));
            }
        }
    }
    assert_eq!(read_peak, 8);
    assert_eq!(write_peak, 1);
    assert_eq!(
        fs::read(root.join("append")).unwrap(),
        vec!["."; 200].join("\n").as_bytes()
    );
    assert_eq!(fs::read_to_string(root.join("input")).unwrap(), text);
    // Preparation of a mutating request must still leave fixture bytes intact.
    let call = registry
        .prepare(
            "denied",
            "call",
            "append",
            &json!({"path":"append","content":"NEVER"}),
            true,
        )
        .unwrap();
    let plan = files
        .prepare_queued(&call, &scope, &cancel, control(&cancel).deadline)
        .unwrap();
    assert_eq!(fs::read(root.join("append")).unwrap().len(), 399);
    drop(plan);
    let executable = std::env::current_exe().unwrap();
    let process_host = executable
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ag1-process-host");
    let processes = Arc::new(ProcessSupervisor::new(&process_host).unwrap());
    let shell = ShellService::new(
        workspace.clone(),
        processes.clone(),
        &spools,
        ShellConfig {
            executable: "/bin/bash".into(),
            env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            command_prefix: None,
            command_template: None,
            sandbox: Sandbox::Off,
            bubblewrap: None,
            output_bytes: 8 * 1024 * 1024,
            address_space: None,
        },
    )
    .unwrap();
    let spool_text = "x".repeat(4 * 1024 * 1024);
    let spool_hash = digest(spool_text.as_bytes());
    let mut spool_times = vec![];
    for sample in 0..5 {
        let operation = format!("spool-{sample}");
        let call = registry
            .prepare(
                &operation,
                "shell-call",
                "bash",
                &json!({"command":"head -c 4194304 /dev/zero | tr '\\000' x"}),
                true,
            )
            .unwrap();
        let plan = shell.prepare(&call, &registry, &scope).unwrap();
        let target = plan.target_revision().to_owned();
        let ticket = authorize(plan.call().clone(), &scope, &target);
        let mut port = context(&store, &operation);
        let (out, ns) = timed(|| {
            executor::execute_with_artifacts(
                &store,
                &mut port,
                ticket,
                execution(&registry, &scope, &target),
                &mut NoHooks,
                |ticket, port| {
                    shell.execute(
                        plan,
                        ticket,
                        ShellExecution {
                            registry: &registry,
                            scope: &scope,
                            policy_revision: 1,
                            now: || 1000,
                            cancel: &cancel,
                            deadline: control(&cancel).deadline,
                        },
                        port,
                        |_| {},
                    )
                },
                |_| {},
            )
            .unwrap()
        });
        let id = out.result["details"]["fullOutputArtifact"]["id"]
            .as_str()
            .unwrap();
        let artifact = port.get(id).unwrap();
        assert_eq!(artifact.sha256, spool_hash);
        assert!(out.result.to_string().len() < 128 * 1024);
        spool_times.push(ns);
    }
    assert_eq!(fs::read_dir(&spools).unwrap().count(), 0);
    let config = root.join(".ag1");
    fs::create_dir_all(config.join("extensions")).unwrap();
    let entry_path = config.join("extensions/measure.ts");
    let code = "export default api => api.registerCommand('noop', {handler: async () => {}});";
    fs::write(&entry_path, code).unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let host_service = ExtensionService::new(
        workspace.clone(),
        ResourceFs::new(vec![workspace.clone()]).unwrap(),
        processes,
        extension_host::Config {
            node: project.join(".local/toolchains/node-v24.14.0-linux-x64/bin/node"),
            host_script: project.join("product/compat/extensions/host.mjs"),
            spools: spools.clone(),
            env: BTreeMap::new(),
        },
    )
    .unwrap();
    let entry = ExtensionResource {
        path: entry_path.to_str().unwrap().into(),
        sha256: digest(code.as_bytes()),
        source_info: SourceInfo {
            path: entry_path.to_str().unwrap().into(),
            source: "auto".into(),
            scope: "project".into(),
            origin: "top-level".into(),
            base_dir: Some(config.to_str().unwrap().into()),
        },
        trust: Trust::UntrustedData,
    };
    let prepared = host_service
        .prepare(
            &scope,
            &[entry],
            trust::Policy {
                project_config_dir: config.to_str().unwrap(),
                canonical_project_dir: config.to_str().unwrap(),
                explicit: Some(true),
                environment: None,
                user_trusted_projects: &[],
            },
            &control(&cancel),
        )
        .unwrap();
    let (mut host, startup) = timed(|| {
        host_service
            .start(prepared, &scope, &control(&cancel))
            .unwrap()
    });
    host.call(
        &scope,
        "bind",
        &json!({"snapshot":{}}),
        &control(&cancel),
        &mut DenyBridge,
    )
    .unwrap();
    let mut roundtrip = vec![];
    for _ in 0..25 {
        let (_, ns) = timed(|| {
            host.call(
                &scope,
                "command",
                &json!({"name":"noop","arguments":""}),
                &control(&cancel),
                &mut DenyBridge,
            )
            .unwrap()
        });
        roundtrip.push(ns);
    }
    host.stop().unwrap();
    assert!(!host.is_alive());
    let report = json!({"status":"measured","runtime":"rust","scope":"Owned synthetic tool workloads; native adapter I/O timing excludes P05, durable dispatch and shell timing include P05; parallel timing includes thread creation. No desktop or model throughput claim.","model_actions":0,"external_network_actions":0,"samples":25,
        "noop_prepare":{"ns":prepare,"calls_per_sample":1000},"durable_noop":{"ns":durable},
        "sequential_reads":{"ns":sequential,"calls_per_sample":8},"parallel_reads":{"ns":parallel,"calls_per_sample":8,"peak_held_proposals":read_peak},"serialized_mutations":{"ns":mutations,"calls_per_sample":8,"peak_held_proposals":write_peak,"final_bytes":399},
        "input_bytes":text.len(),"input_sha256":digest(text.as_bytes()),"read_result_sha256":read_hash,
        "large_spool":{"ns":spool_times,"bytes_per_sample":spool_text.len(),"sha256":spool_hash,"spools_cleaned":true},"extension":{"startup_ns":startup,"roundtrip_ns":roundtrip,"stopped":true},"no_effect_before_approval":true});
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&args[2])
        .unwrap();
    output
        .write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes())
        .unwrap();
}
