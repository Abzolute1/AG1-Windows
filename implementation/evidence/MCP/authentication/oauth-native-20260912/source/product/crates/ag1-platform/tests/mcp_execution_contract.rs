#![cfg(target_os = "linux")]
use ag1_core::{
    Clock, DomainError, SessionId,
    context::{ContextArtifact, features::Features},
    tools::{
        approval::{AuthorizedCall, Authorizer, ExecutionFence, Preflight, PreflightContext},
        executor::{
            self, Effect, Execution, ExecutionArtifacts, NoHooks, Outcome, Status, ToolEvent,
        },
        policy::Policy,
        registry::{Registry, Scope},
        result::ResultBudget,
    },
};
use ag1_platform::mcp::{
    Revision,
    catalog::CatalogScope,
    definitions::{Availability, Configuration, Draft, Provenance},
    discovery::DiscoveryLimits,
    manager::{ConnectionConsent, ConnectionHealth, Manager, SelectedTool, Selection},
};
use ag1_storage::{
    Boundary, FaultInjector, StorageError, StorageResult, Store, context::ContextStore,
};
use serde_json::{Value, json};
#[path = "mcp_private_execution/mod.rs"]
mod private_execution;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Fault(AtomicUsize);
impl FaultInjector for Fault {
    fn check(&self, boundary: Boundary) -> StorageResult<()> {
        if boundary == Boundary::BeforeAck
            && self
                .0
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                .ok()
                == Some(1)
        {
            return Err(StorageError::Injected(boundary));
        }
        Ok(())
    }
}
const NOW: &str = "2026-09-12T00:00:00.000Z";
struct TestClock;
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        100
    }
}
struct Peer(Child);
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Case {
    manager: Arc<Manager>,
    _peer: Option<Peer>,
    dir: tempfile::TempDir,
    store: Store,
    fault: Arc<Fault>,
    server: String,
    connection: String,
    registry: Registry,
    selection: Selection,
    scope: Scope,
}
fn until(mut check: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(8);
    while !check() {
        assert!(Instant::now() < end, "owned fixture deadline");
        std::thread::sleep(Duration::from_millis(5));
    }
}
impl Case {
    fn new(transport: &str, revision: Revision, mode: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(
            path.join("peer.py"),
            if mode.starts_with("content_") {
                include_str!("fixtures/mcp_content.py")
            } else {
                include_str!("fixtures/mcp_operation.py")
            },
        )
        .unwrap();
        std::fs::write(path.join("mode"), mode).unwrap();
        let args = vec![
            "-I".into(),
            path.join("peer.py").to_str().unwrap().into(),
            transport.into(),
            revision.as_str().into(),
            path.to_str().unwrap().into(),
        ];
        let mut peer = None;
        let (configuration, consent) = if transport == "stdio" {
            (
                Configuration::Local {
                    executable: "/usr/bin/python3".into(),
                    arguments: args,
                    cwd: path.to_str().unwrap().into(),
                    public_environment: BTreeMap::new(),
                    secret_environment: BTreeSet::new(),
                },
                ConnectionConsent::LocalHostAccountExecution,
            )
        } else {
            peer = Some(Peer(
                Command::new("/usr/bin/python3")
                    .args(args)
                    .env_clear()
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::from(
                        std::fs::File::create(path.join("peer.stderr")).unwrap(),
                    ))
                    .spawn()
                    .unwrap(),
            ));
            until(|| path.join("port").exists());
            let port = std::fs::read_to_string(path.join("port")).unwrap();
            (
                Configuration::Http {
                    url: format!("http://127.0.0.1:{port}/mcp"),
                    loopback_development: true,
                },
                ConnectionConsent::LoopbackDevelopmentEndpoint,
            )
        };
        let manager = Manager::open(
            Store::open(&path.join("manager")).unwrap(),
            Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
            DiscoveryLimits::new(Duration::from_millis(800)).unwrap(),
        )
        .unwrap();
        let server = manager
            .add_drafts(vec![Draft {
                name: "owned fixture".into(),
                configuration,
                protocol: revision.as_str().into(),
                auth_profile: None,
                provenance: Provenance::Manual,
            }])
            .unwrap()
            .remove(0);
        manager
            .set_availability(
                &server,
                1,
                Availability {
                    enabled: true,
                    general_chat: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let connection = manager
            .connect(
                manager
                    .review_connection(&server, CatalogScope::GeneralChat)
                    .unwrap(),
                consent,
            )
            .unwrap();
        until(|| {
            let status = manager.status(&connection).unwrap();
            assert_ne!(
                status.health,
                ConnectionHealth::Failed,
                "{transport}/{mode}: {status:?}; stderr={:?}; events={:?}",
                std::fs::read_to_string(path.join("peer.stderr")),
                std::fs::read_to_string(path.join("events.jsonl"))
            );
            status.health == ConnectionHealth::Ready
        });
        let mut registry = Registry::builtins().unwrap();
        let selection = if mode.starts_with("content_") {
            manager
                .select_capabilities(
                    &mut registry,
                    CatalogScope::GeneralChat,
                    &[ag1_platform::mcp::manager::SelectedCapability {
                        connection_id: connection.clone(),
                        kind: ag1_core::tools::mcp::Kind::Resource,
                        key: "fixture://owned/source".into(),
                    }],
                )
                .unwrap()
        } else {
            manager
                .select_tools(
                    &mut registry,
                    CatalogScope::GeneralChat,
                    &[SelectedTool {
                        connection_id: connection.clone(),
                        original_name: "owned".into(),
                    }],
                )
                .unwrap()
        };
        let fault = Arc::new(Fault(AtomicUsize::new(0)));
        let store = Store::open_with(
            &path.join("conversation"),
            Arc::new(TestClock),
            fault.clone(),
        )
        .unwrap();
        store.create_session("session",json!({"type":"session","version":3,"id":"session","cwd":"/fixture","timestamp":NOW}).to_string().as_bytes()).unwrap();
        store
            .register_origin("origin", "session", &[4; 32])
            .unwrap();
        Self {
            manager: Arc::new(manager),
            _peer: peer,
            dir,
            store,
            fault,
            server,
            connection,
            registry,
            selection,
            scope: Scope {
                workspace: "workspace".into(),
                session: "session".into(),
                epoch: 1,
                revision: 0,
            },
        }
    }
    fn port(&self, operation: &str) -> ContextStore<'_> {
        ContextStore::new(
            &self.store,
            SessionId::new("session").unwrap(),
            "origin",
            operation,
            NOW,
            Features::default(),
        )
        .unwrap()
    }
    fn ticket(&self, operation: &str, ttl: u64, args: &Value) -> AuthorizedCall {
        let name = self.selection.wire_names().remove(0);
        let call = self
            .selection
            .prepare(
                &self.manager,
                &self.registry,
                operation,
                "call",
                &name,
                args.to_string().as_bytes(),
            )
            .unwrap();
        let target = self.selection.target_revision(&name).unwrap();
        let policy: Policy = serde_json::from_value(json!({"mode":"disabled","cwd":""})).unwrap();
        let mut authorizer = Authorizer::new("owned-authorizer").unwrap();
        let Preflight::Approval(approval) = authorizer
            .preflight(
                call,
                PreflightContext {
                    scope: &self.scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 100,
                    ttl,
                    untrusted_extensions: false,
                },
            )
            .unwrap()
        else {
            panic!("must ask")
        };
        authorizer
            .answer(
                &approval.binding,
                true,
                &self.registry,
                ExecutionFence {
                    scope: &self.scope,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 100,
                },
            )
            .unwrap()
            .unwrap()
    }
    fn run(
        &self,
        operation: &str,
        ttl: u64,
        args: &Value,
        events: impl FnMut(ToolEvent),
    ) -> Result<Outcome, DomainError> {
        let ticket = self.ticket(operation, ttl, args);
        self.execute(operation, ticket, events)
    }
    fn execute(
        &self,
        operation: &str,
        ticket: AuthorizedCall,
        events: impl FnMut(ToolEvent),
    ) -> Result<Outcome, DomainError> {
        let target = ticket.target_revision().to_owned();
        self.manager.execute_tool(
            &self.store,
            &mut self.port(operation),
            &self.selection,
            ticket,
            Execution {
                registry: &self.registry,
                fence: ExecutionFence {
                    scope: &self.scope,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 100,
                },
                budget: ResultBudget::default(),
                clock: &TestClock,
            },
            &mut NoHooks,
            events,
        )
    }
    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|r| r["method"] == "tools/call")
            .collect()
    }
    fn original(&self, outcome: &Outcome) -> Value {
        let artifact: ContextArtifact =
            serde_json::from_value(outcome.result["details"]["mcp"]["originalResponse"].clone())
                .unwrap();
        self.port("reader").read_json(&artifact).unwrap()
    }
}
#[test]
fn both_transports_and_revisions_execute_only_after_durable_intent_and_replay_receipt_once() {
    for transport in ["stdio", "http"] {
        for revision in [Revision::Legacy20251125, Revision::Current20260728] {
            let case = Case::new(transport, revision, "structured");
            let mut events = Vec::new();
            let first = case
                .run(
                    "operation",
                    10000,
                    &json!({"value":7,"text":"literal $() ;"}),
                    |event| {
                        if event.stage == "start" {
                            assert!(case.calls().is_empty());
                            let record =
                                executor::recover(&case.store, "workspace", "session", "operation")
                                    .unwrap()
                                    .unwrap();
                            assert_eq!(record.status, Status::Intent);
                            assert!(record.mcp.is_some());
                        }
                        events.push(event.stage);
                    },
                )
                .unwrap();
            assert_eq!(events, vec!["start", "end"]);
            assert_eq!(first.record.status, Status::Completed);
            assert_eq!(first.record.effect, Effect::Unknown);
            assert_eq!(first.result["isError"], false);
            assert_eq!(first.result["details"]["mcp"]["outcome"], "complete");
            assert_eq!(
                case.original(&first)["result"]["structuredContent"],
                json!({"value":7})
            );
            assert!(case.original(&first)["result"]["details"]["passed"] == true);
            assert!(!first.result.to_string().contains("nativeVerification"));
            assert!(!first.result.to_string().contains("private-state"));
            let replay = case
                .run(
                    "operation",
                    10000,
                    &json!({"value":7,"text":"literal $() ;"}),
                    |_| panic!("replay must not emit events"),
                )
                .unwrap();
            assert!(replay.replayed);
            assert_eq!(case.calls().len(), 1);
            assert_eq!(case.calls()[0]["params"]["name"], "owned");
            assert_eq!(
                case.calls()[0]["params"]["arguments"]["text"],
                "literal $() ;"
            );
        }
    }
}
#[test]
fn application_protocol_schema_and_input_errors_retain_originals_without_native_claims() {
    for transport in ["stdio", "http"] {
        for (mode, code) in [
            ("application_error", "application_error"),
            ("protocol_error", "protocol_error"),
            ("bad_output", "protocol_error"),
            ("missing_output", "protocol_error"),
            ("invalid_content", "protocol_error"),
            ("invalid_media", "protocol_error"),
            ("invalid_uri", "protocol_error"),
            ("invalid_error", "protocol_error"),
            ("missing_complete", "protocol_error"),
            ("input_required", "unsupported_input"),
        ] {
            let case = Case::new(transport, Revision::Current20260728, mode);
            let result = case
                .run("operation", 10000, &json!({"value":7}), |_| {})
                .unwrap();
            assert_eq!(
                result.result["details"]["mcp"]["outcome"], code,
                "{transport}/{mode}"
            );
            assert_eq!(result.result["isError"], true);
            assert_eq!(result.record.effect, Effect::Unknown);
            let raw = case.original(&result);
            if mode == "input_required" {
                assert!(!raw.to_string().contains("private-state"));
                assert!(raw["result"].get("inputRequests").is_none());
                assert_eq!(
                    result.result["details"]["mcp"]["completeResponseRetained"],
                    false
                );
            }
            assert!(raw.get("result").is_some() || raw.get("error").is_some());
            assert!(!result.result.to_string().contains("private-state"));
            assert!(
                !result
                    .result
                    .to_string()
                    .contains("synthetic-private-error")
            );
            assert_eq!(case.calls().len(), 1);
        }
    }
}
#[test]
fn current_scalar_array_and_null_results_preserve_shape_and_legacy_refuses_scalar() {
    for (revision, mode, expected) in [
        (Revision::Current20260728, "scalar", json!(7)),
        (Revision::Current20260728, "array", json!([1, 2])),
        (Revision::Current20260728, "null", Value::Null),
        (Revision::Legacy20251125, "scalar", json!(7)),
    ] {
        let case = Case::new("stdio", revision, mode);
        let result = case
            .run("operation", 10000, &json!({"value":7}), |_| {})
            .unwrap();
        assert_eq!(
            case.original(&result)["result"]["structuredContent"],
            expected
        );
        assert_eq!(
            result.result["isError"],
            revision == Revision::Legacy20251125
        );
    }
}
#[test]
fn mixed_and_large_results_use_inert_projection_with_complete_original_custody() {
    for mode in ["mixed", "large"] {
        let case = Case::new("stdio", Revision::Current20260728, mode);
        let result = case
            .run(
                "operation",
                10000,
                &json!({"value":7,"text":"x".repeat(12_000)}),
                |_| {},
            )
            .unwrap();
        assert_eq!(result.result["isError"], false);
        let raw = case.original(&result);
        if mode == "large" {
            assert!(
                raw["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .ends_with("END")
            );
            assert!(result.result.to_string().len() < 30_000);
        } else {
            assert_eq!(raw["result"]["content"].as_array().unwrap().len(), 5);
            assert!(
                result.result["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|b| b["type"] == "text")
            );
        }
    }
}
#[test]
fn schema_change_expiry_and_unsupported_task_refuse_before_tool_send() {
    for transport in ["stdio", "http"] {
        for mode in ["schema_change", "expiry", "task_required"] {
            let case = Case::new(transport, Revision::Current20260728, mode);
            let result = case
                .run(
                    "operation",
                    if mode == "expiry" { 50 } else { 10000 },
                    &json!({"value":7}),
                    |_| {},
                )
                .unwrap();
            assert_eq!(result.result["isError"], true);
            assert_eq!(result.record.effect, Effect::None);
            assert_eq!(result.result["details"]["mcp"]["sendAdmitted"], false);
            assert!(case.calls().is_empty());
        }
    }
}
#[test]
fn disable_after_intent_and_during_refresh_prevents_tool_send() {
    for transport in ["stdio", "http"] {
        for during_refresh in [false, true] {
            let case = Case::new(transport, Revision::Current20260728, "slow_refresh");
            std::thread::scope(|threads| {
                let run = threads.spawn(|| {
                    case.run("operation", 10000, &json!({"value":7}), |event| {
                        if event.stage == "start" && !during_refresh {
                            case.manager
                                .set_availability(&case.server, 2, Availability::default())
                                .unwrap();
                        }
                    })
                    .unwrap()
                });
                if during_refresh {
                    until(|| case.dir.path().join("refreshing").exists());
                    case.manager
                        .set_availability(&case.server, 2, Availability::default())
                        .unwrap();
                }
                let result = run.join().unwrap();
                assert_eq!(result.record.effect, Effect::None);
                assert!(case.calls().is_empty());
            });
        }
    }
}
#[test]
fn timeout_disconnect_truncation_and_oversize_settle_uncertainty_without_replay() {
    for transport in ["stdio", "http"] {
        for mode in ["hang", "truncated", "oversized"] {
            let case = Case::new(transport, Revision::Current20260728, mode);
            let result = case
                .run("operation", 10000, &json!({"value":7}), |_| {})
                .unwrap();
            assert_eq!(result.record.status, Status::Completed);
            assert_eq!(result.record.effect, Effect::Unknown);
            assert_eq!(result.result["isError"], true);
            assert_eq!(case.calls().len(), 1);
            let record = executor::recover(&case.store, "workspace", "session", "operation")
                .unwrap()
                .unwrap();
            assert_eq!(record.effect, Effect::Unknown);
        }
    }
}
#[test]
fn in_flight_disconnect_is_bounded_and_second_call_cannot_enter_the_queue() {
    for transport in ["stdio", "http"] {
        let case = Case::new(transport, Revision::Current20260728, "hang");
        std::thread::scope(|threads| {
            let run = threads.spawn(|| {
                case.run("operation", 10000, &json!({"value":7}), |_| {})
                    .unwrap()
            });
            until(|| case.dir.path().join("effects").exists());
            assert!(matches!(
                case.run("other", 10000, &json!({"value":7}), |_| {}),
                Err(DomainError::QueueFull)
            ));
            assert!(
                executor::recover(&case.store, "workspace", "session", "other")
                    .unwrap()
                    .is_none()
            );
            let start = Instant::now();
            case.manager.disconnect(&case.connection).unwrap();
            let result = run.join().unwrap();
            assert!(start.elapsed() < Duration::from_secs(2));
            assert_eq!(result.result["details"]["mcp"]["outcome"], "cancelled");
            assert_eq!(result.record.effect, Effect::Unknown);
            case.manager.wait_disconnected(&case.connection).unwrap();
        });
    }
}
#[test]
fn unsupported_legacy_input_is_explicitly_refused_without_continuation() {
    let case = Case::new("stdio", Revision::Legacy20251125, "legacy_request");
    let result = case
        .run("operation", 10000, &json!({"value":7}), |_| {})
        .unwrap();
    assert_eq!(result.result["isError"], false);
    assert!(case.dir.path().join("input_refused").exists());
    assert_eq!(case.calls().len(), 1);
}

#[test]
fn http_sse_results_are_correlated_and_change_notifications_retire_the_selection() {
    for revision in [Revision::Legacy20251125, Revision::Current20260728] {
        for mode in ["sse", "changed_during_call"] {
            let case = Case::new("http", revision, mode);
            let result = case
                .run("operation", 10000, &json!({"value":7}), |_| {})
                .unwrap();
            assert_eq!(result.result["isError"], false);
            assert_eq!(
                case.original(&result)["result"]["content"][0]["text"],
                "owned result"
            );
            if mode == "changed_during_call" {
                assert!(
                    case.selection
                        .validate(&case.manager, &case.registry)
                        .is_err()
                );
            }
            assert_eq!(case.calls().len(), 1);
        }
    }
}
#[test]
fn uncorrelated_replies_never_become_success_or_get_retried() {
    for transport in ["stdio", "http"] {
        let case = Case::new(transport, Revision::Current20260728, "wrong_id");
        let result = case
            .run("operation", 10000, &json!({"value":7}), |_| {})
            .unwrap();
        assert_eq!(result.result["isError"], true);
        assert_eq!(result.result["details"]["mcp"]["outcome"], "protocol_error");
        assert_eq!(result.record.effect, Effect::Unknown);
        assert_eq!(case.calls().len(), 1);
    }
}
#[test]
fn failed_intent_or_original_ack_cannot_send_or_repeat_a_tool_effect() {
    for transport in ["stdio", "http"] {
        for after_send in [false, true] {
            let case = Case::new(transport, Revision::Current20260728, "structured");
            if !after_send {
                case.fault.0.store(2, Ordering::Release);
            }
            let mut events = Vec::new();
            let result = case.run("operation", 10000, &json!({"value":7}), |event| {
                if after_send && event.stage == "start" {
                    case.fault.0.store(1, Ordering::Release);
                }
                events.push(event.stage);
            });
            assert!(result.is_err());
            assert!(!events.contains(&"end".to_owned()));
            assert_eq!(case.calls().len(), usize::from(after_send));
            let record = executor::recover(&case.store, "workspace", "session", "operation")
                .unwrap()
                .unwrap();
            assert_eq!(record.status, Status::Intent);
            assert_eq!(record.effect, Effect::Unknown);
            assert!(matches!(
                case.run("operation", 10000, &json!({"value":7}), |_| panic!(
                    "uncertain replay"
                )),
                Err(DomainError::DurabilityUnknown)
            ));
            assert_eq!(case.calls().len(), usize::from(after_send));
        }
    }
}

#[test]
fn oversized_registry_snapshot_is_refused_before_intent_or_send() {
    let mut case = Case::new("stdio", Revision::Current20260728, "structured");
    for i in 0..18 {
        case.registry.register(json!({"name":format!("owned_{i}"),"description":"x".repeat(240*1024),"parameters":{"type":"object"}}), ag1_core::tools::registry::Origin::TrustedHost).unwrap();
    }
    case.selection = case
        .manager
        .select_tools(
            &mut case.registry,
            CatalogScope::GeneralChat,
            &[SelectedTool {
                connection_id: case.connection.clone(),
                original_name: "owned".into(),
            }],
        )
        .unwrap();
    assert!(matches!(
        case.run("operation", 10000, &json!({"value":7}), |_| panic!(
            "must not start"
        )),
        Err(DomainError::LimitExceeded(_))
    ));
    assert!(case.calls().is_empty());
    assert!(
        executor::recover(&case.store, "workspace", "session", "operation")
            .unwrap()
            .is_none()
    );
}

mod mcp_content;

mod mcp_runtime;
