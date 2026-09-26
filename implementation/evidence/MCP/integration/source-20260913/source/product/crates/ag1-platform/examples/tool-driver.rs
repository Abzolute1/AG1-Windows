//! Deterministic P09 fixture host. Uses real native file adapters, approvals and P05 persistence.
use ag1_core::{
    Clock, DomainError, SessionId,
    context::features::Features,
    tools::{
        approval::{Authorizer, ExecutionFence, Preflight, PreflightContext},
        executor::{self, AdapterOutcome, Effect, Execution, Invocation, NoHooks, Rejection},
        policy::Policy,
        registry::{Registry, Scope},
        result::ResultBudget,
    },
};
use ag1_platform::{
    file_tools::{FileExecution, FileService},
    process::ProcessSupervisor,
    search_tools::SearchService,
    shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
    workspace::WorkspaceService,
};
use ag1_storage::{StorageError, Store, context::ContextStore};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
struct FixtureClock;
impl Clock for FixtureClock {
    fn now_ms(&self) -> u64 {
        1000
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return Err(
            "tool-driver <fixture-workspace> <private-store> <requests.json> <new-results.jsonl>"
                .into(),
        );
    }
    let root = Path::new(&args[1])
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if fs::read(root.join(".ag1-tool-fixture")).map_err(|e| e.to_string())?
        != b"P09 isolated tool fixture\n"
    {
        return Err("explicit isolated fixture marker required".into());
    }
    let raw = fs::read(&args[3]).map_err(|e| e.to_string())?;
    if raw.len() > 8 * 1024 * 1024 {
        return Err("fixture request bytes".into());
    }
    let requests: Vec<Value> = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    if requests.is_empty() || requests.len() > 128 {
        return Err("fixture request count".into());
    }
    let mut output = BufWriter::new(
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&args[4])
            .map_err(|e| e.to_string())?,
    );
    let workspace = WorkspaceService::new()
        .map_err(|e| format!("{e:?}"))?
        .register(&root)
        .map_err(|e| format!("{e:?}"))?;
    let files = FileService::new(workspace.clone());
    let mut registry = Registry::builtins().map_err(|e| format!("{e:?}"))?;
    let names = [
        "write",
        "edit",
        "append",
        "editPreview",
        "editApply",
        "read",
        "ls",
        "grep",
        "find",
        "bash",
    ]
    .map(str::to_string);
    registry
        .activate(&names, &names.iter().cloned().collect())
        .map_err(|e| format!("{e:?}"))?;
    let store = Store::open(Path::new(&args[2])).map_err(|e| e.to_string())?;
    let timestamp = "2026-09-05T00:00:00.000Z";
    match store.snapshot("fixture") {
        Ok(_) => {}
        Err(StorageError::Missing(_)) => {
            store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":root,"timestamp":timestamp}).to_string().as_bytes()).map_err(|e|e.to_string())?;
            store
                .register_origin("fixture", "fixture", &[6; 32])
                .map_err(|e| e.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    let scope = Scope {
        workspace: workspace.id().as_str().into(),
        session: "fixture".into(),
        epoch: 1,
        revision: 0,
    };
    let spools = Path::new(&args[2]).join("tool-spools");
    fs::create_dir_all(&spools).map_err(|e| e.to_string())?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let process_host = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("fixture executable directory")?
        .join("ag1-process-host");
    let processes = Arc::new(ProcessSupervisor::new(&process_host).map_err(|e| format!("{e:?}"))?);
    let search = SearchService::new(
        workspace.clone(),
        processes.clone(),
        "/usr/bin/rg".as_ref(),
        "/usr/bin/fd".as_ref(),
        &spools,
    )
    .map_err(|e| format!("{e:?}"))?;
    let shell = ShellService::new(
        workspace.clone(),
        processes,
        &spools,
        ShellConfig {
            executable: "/bin/bash".into(),
            env: [("PATH".into(), "/usr/bin:/bin".into())].into(),
            command_prefix: None,
            command_template: None,
            sandbox: Sandbox::Off,
            bubblewrap: None,
            output_bytes: 2 * 1024 * 1024,
            address_space: Some(1024 * 1024 * 1024),
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    let clock = FixtureClock;
    for request in requests {
        let operation = request["operation"].as_str().ok_or("operation required")?;
        let name = request["name"].as_str().ok_or("name required")?;
        let arguments = &request["arguments"];
        let complete = request["complete"].as_bool().unwrap_or(true);
        let mut artifacts = ContextStore::new(
            &store,
            SessionId::new("fixture").unwrap(),
            "fixture",
            operation,
            timestamp,
            Features::default(),
        )
        .map_err(|e| format!("{e:?}"))?;
        let mut events = vec![];
        let cancel = AtomicBool::new(request["cancelBefore"] == true);
        let deadline = Instant::now() + Duration::from_secs(5);
        let attempt = (|| -> Result<executor::Outcome, DomainError> {
            let call = registry.prepare(operation, operation, name, arguments, complete)?;
            let call = executor::prepare(&registry, &call, &mut NoHooks)?;
            if let Some(replayed) = executor::replay(&store, &artifacts, &call, &scope)? {
                return Ok(replayed);
            }
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            if matches!(name, "grep" | "find") {
                files.relative_path(
                    arguments["path"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("."),
                )?;
            }
            let shell_plan = if name == "bash" {
                Some(shell.prepare(&call, &registry, &scope)?)
            } else {
                None
            };
            let call = shell_plan
                .as_ref()
                .map(|p| p.call().clone())
                .unwrap_or(call);
            let prepared = if matches!(name, "ls" | "grep" | "find" | "bash") {
                None
            } else {
                Some(files.prepare_queued(&call, &scope, &cancel, deadline)?)
            };
            let target = prepared
                .as_ref()
                .map(|p| p.target_revision().to_string())
                .unwrap_or_else(|| {
                    if let Some(plan) = &shell_plan {
                        plan.target_revision().into()
                    } else if name == "ls" {
                        files.list_target_revision()
                    } else {
                        search.target_revision()
                    }
                });
            let policy: Policy = serde_json::from_value(
                json!({"mode":request["mode"].as_str().unwrap_or("interactive"),"cwd":root}),
            )
            .map_err(|_| DomainError::InvalidInput("policy mode".into()))?;
            let mut authorizer = Authorizer::new("fixture-driver")?;
            let preflight = authorizer.preflight(
                call,
                PreflightContext {
                    scope: &scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 1000,
                    ttl: 1000,
                    untrusted_extensions: false,
                },
            )?;
            let ticket = match preflight {
                Preflight::Ready(ticket) => ticket,
                Preflight::Rejected(_) => return Err(DomainError::PermissionDenied),
                Preflight::Approval(pending) => authorizer
                    .answer(
                        &pending.binding,
                        request["approve"] == true,
                        &registry,
                        ExecutionFence {
                            scope: &scope,
                            policy_revision: 1,
                            target_revision: &target,
                            now: 1000,
                        },
                    )?
                    .ok_or(DomainError::PermissionDenied)?,
            };
            executor::execute_with_artifacts(
                &store,
                &mut artifacts,
                ticket,
                Execution {
                    registry: &registry,
                    fence: ExecutionFence {
                        scope: &scope,
                        policy_revision: 1,
                        target_revision: &target,
                        now: 1000,
                    },
                    budget: ResultBudget::default(),
                    clock: &clock,
                },
                &mut NoHooks,
                |ticket, artifacts| {
                    if let Some(plan) = shell_plan {
                        return shell.execute(
                            plan,
                            ticket,
                            ShellExecution {
                                registry: &registry,
                                scope: &scope,
                                policy_revision: 1,
                                now: || 1000,
                                cancel: &cancel,
                                deadline,
                            },
                            artifacts,
                            |_| {},
                        );
                    }
                    let execution = FileExecution {
                        registry: &registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || 1000,
                        cancel: &cancel,
                        deadline,
                    };
                    let out = if let Some(prepared) = prepared {
                        files.execute(prepared, ticket, execution)?
                    } else if name == "ls" {
                        files.execute_list(ticket, execution)?
                    } else {
                        search.execute(ticket, execution)?
                    };
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
                |e| events.push(e),
            )
        })();
        let outcome = match attempt {
            Ok(outcome) => {
                json!({"operation":operation,"record":outcome.record,"result":outcome.result,"replayed":outcome.replayed,"events":events})
            }
            Err(error) => {
                // A recorded intent may have had effects. Never relabel it as a pre-execution rejection.
                if executor::recover(&store, &scope.workspace, &scope.session, operation)
                    .map_err(|e| format!("{e:?}"))?
                    .is_some_and(|record| record.status != executor::Status::Rejected)
                {
                    json!({"operation":operation,"error":format!("{error:?}"),"events":events})
                } else {
                    let hint = arguments["path"].as_str().and_then(|path| {
                        let mut check = || {
                            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                                Err(DomainError::Cancelled)
                            } else if Instant::now() >= deadline {
                                Err(DomainError::TimedOut)
                            } else {
                                Ok(())
                            }
                        };
                        (if matches!(name, "grep" | "find") {
                            search.rejection_hint(&error, path, &mut check)
                        } else {
                            files.rejection_hint(&error, path, &mut check)
                        })
                        .ok()
                        .flatten()
                    });
                    let outcome = executor::reject_with_hint(
                        &store,
                        &mut artifacts,
                        Invocation {
                            operation,
                            call_id: operation,
                            name,
                            arguments,
                            complete,
                        },
                        Rejection {
                            scope: &scope,
                            reason: &error,
                            clock: &clock,
                            budget: ResultBudget::default(),
                        },
                        hint.as_deref(),
                        |e| events.push(e),
                    )
                    .map_err(|e| format!("{e:?}"))?;
                    json!({"operation":operation,"record":outcome.record,"result":outcome.result,"replayed":outcome.replayed,"events":events})
                }
            }
        };
        serde_json::to_writer(&mut output, &outcome).map_err(|e| e.to_string())?;
        output.write_all(b"\n").map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
    }
    output.get_ref().sync_all().map_err(|e| e.to_string())?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
