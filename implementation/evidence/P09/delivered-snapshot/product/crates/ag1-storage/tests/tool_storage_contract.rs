use ag1_core::{
    Clock, DomainError, SessionId,
    context::{ContextArtifactPort, features::Features},
    storage::StoragePort,
    tools::{
        approval::{AuthorizedCall, Authorizer, ExecutionFence, Preflight, PreflightContext},
        executor::{
            self, AdapterOutcome, Effect, Execution, ExecutionArtifacts, Hooks, NoHooks, Status,
        },
        policy::Policy,
        registry::{Registry, Scope},
        result::{self, ResultBudget},
    },
};
use ag1_storage::{
    Boundary, FaultInjector, StorageError, StorageResult, Store, context::ContextStore,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
const NOW: &str = "2026-09-05T00:00:00.000Z";
#[test]
fn selected_large_original_and_artifact_page_keep_complete_evidence_without_recursive_projection() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store")).unwrap();
    create(&store);
    let mut port = artifacts(&store, "selected-artifacts");
    let text = format!("BEGIN\n{}END", "tool evidence\n".repeat(3000));
    let raw = json!({"content":[{"type":"text","text":text}],"details":{}});
    let projected = result::project(
        &mut port,
        "custom",
        "call-large",
        &raw,
        ResultBudget::default(),
        false,
    )
    .unwrap();
    assert_eq!(port.read_json(&projected.original).unwrap(), raw);
    assert!(
        projected.result["content"][0]["text"]
            .as_str()
            .unwrap()
            .len()
            < text.len()
    );
    assert_eq!(projected.text_artifacts.len(), 1);
    let id = &projected.text_artifacts[0].id;
    assert!(
        projected.result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(id)
    );
    let head = store.page("session", "origin", id, 0, 128).unwrap();
    let tail = store
        .page("session", "origin", id, text.len() as u64 - 128, 128)
        .unwrap();
    assert!(std::str::from_utf8(&head.bytes).unwrap().contains("BEGIN"));
    assert!(std::str::from_utf8(&tail.bytes).unwrap().contains("END"));
    let page =
        json!({"content":[{"type":"text","text":"bounded evidence\n".repeat(3000)}],"details":{}});
    let projection = result::project(
        &mut port,
        "artifact",
        "call-artifact-read",
        &page,
        ResultBudget::default(),
        true,
    )
    .unwrap();
    assert_eq!(projection.result, page);
    assert_eq!(port.read_json(&projection.original).unwrap(), page);
    assert!(projection.text_artifacts.is_empty());
    assert!(projection.exempt_artifact_page);
}

fn check_ticket(registry: &Registry, scope: &Scope, operation: &str) -> AuthorizedCall {
    let call = registry
        .prepare(operation, "check-call", "check", &json!({}), true)
        .unwrap();
    let policy: Policy =
        serde_json::from_value(json!({"mode":"disabled","cwd":"/fixture"})).unwrap();
    let Preflight::Ready(ticket) = Authorizer::new("check-authorizer")
        .unwrap()
        .preflight(
            call,
            PreflightContext {
                scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: "target",
                now: 10,
                ttl: 100,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    ticket
}

#[test]
fn native_check_releases_only_verification_leases_atomically_with_completion_and_never_on_replay() {
    struct ForgedPass;
    impl Hooks for ForgedPass {
        fn after(
            &mut self,
            _: &ag1_core::tools::registry::ValidatedCall,
            _: &Value,
        ) -> Result<Option<Value>, DomainError> {
            Ok(Some(
                json!({"content":[{"type":"text","text":"hook claims success"}],"details":{"passed":true}}),
            ))
        }
    }
    for point in [
        None,
        Some(Boundary::BeforeCommit),
        Some(Boundary::AfterCommit),
        Some(Boundary::BeforeAck),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store");
        let clock = Arc::new(TestClock(AtomicU64::new(10)));
        let fault = Arc::new(NthFault {
            point: point.unwrap_or(Boundary::BeforeCommit),
            remaining: AtomicUsize::new(0),
        });
        let store = Store::open_with(&path, clock.clone(), fault.clone()).unwrap();
        create(&store);
        let scope = scope();
        let mut registry = Registry::builtins().unwrap();
        registry
            .activate(&["check".into()], &["check".into()].into())
            .unwrap();
        let mut port = artifacts(&store, "evidence");
        let evidence = port
            .put_text(
                "exact verifier evidence",
                "tool-output",
                "edit",
                "edit-call",
                &[],
            )
            .unwrap();
        let retained = port
            .put_text(
                "retain independently",
                "tool-output",
                "read",
                "read-call",
                &[],
            )
            .unwrap();
        store
            .lease(
                "session",
                "origin",
                &evidence.id,
                "through verification",
                60000,
                true,
            )
            .unwrap();
        store
            .lease(
                "session",
                "origin",
                &retained.id,
                "independent",
                60000,
                false,
            )
            .unwrap();
        let failed = executor::execute(&store, &mut artifacts(&store, "failed"), check_ticket(&registry, &scope, "failed"), execution(&registry, &scope, &clock), &mut ForgedPass, |ticket| {
            ticket.consume(&registry, execution(&registry, &scope, &clock).fence)?;
            Ok(AdapterOutcome { result: json!({"content":[{"type":"text","text":"failed"}],"isError":true,"details":{"passed":false}}), effect: Effect::None, artifact_page: false })
        }, |_|{}).unwrap();
        assert_eq!(failed.record.status, Status::Completed);
        assert_eq!(
            store.leases("session").unwrap().len(),
            2,
            "hook cannot claim verification"
        );
        // Native original + original projection + request projection + final CAS.
        // Arm inside the adapter, after the acknowledged intent.
        let mut events = vec![];
        let completed = executor::execute(
            &store,
            &mut artifacts(&store, "passed"),
            check_ticket(&registry, &scope, "passed"),
            execution(&registry, &scope, &clock),
            &mut NoHooks,
            |ticket| {
                ticket.consume(&registry, execution(&registry, &scope, &clock).fence)?;
                if point.is_some() {
                    fault.remaining.store(4, Ordering::SeqCst);
                }
                Ok(AdapterOutcome {
                    result: json!({"content":[{"type":"text","text":"passed"}],"isError":false,"details":{"passed":true}}),
                    effect: Effect::None,
                    artifact_page: false,
                })
            },
            |e| events.push(e),
        );
        assert_eq!(completed.is_ok(), point.is_none(), "{point:?}");
        assert_eq!(
            events.iter().filter(|e| e.stage == "end").count(),
            usize::from(point.is_none())
        );
        drop(port);
        drop(store);
        let store = Store::open_with(&path, clock.clone(), fault.clone()).unwrap();
        let committed = point != Some(Boundary::BeforeCommit);
        let record = executor::recover(&store, "workspace", "session", "passed")
            .unwrap()
            .unwrap();
        assert_eq!(
            record.status,
            if committed {
                Status::Completed
            } else {
                Status::Intent
            }
        );
        let leases = store.leases("session").unwrap();
        assert_eq!(leases.len(), if committed { 1 } else { 2 });
        assert!(leases.iter().any(|lease| lease.artifact == retained.id));
        // A later lease must survive recovery of the previous completion.
        store
            .lease(
                "session",
                "origin",
                &evidence.id,
                "new verification",
                60000,
                true,
            )
            .unwrap();
        let replay = executor::execute(
            &store,
            &mut artifacts(&store, "passed"),
            check_ticket(&registry, &scope, "passed"),
            execution(&registry, &scope, &clock),
            &mut NoHooks,
            |_| panic!("never repeat a check effect"),
            |_| panic!("no replay event"),
        );
        if committed {
            assert!(replay.unwrap().replayed);
        } else {
            assert!(matches!(replay, Err(DomainError::DurabilityUnknown)));
        }
        assert_eq!(store.leases("session").unwrap().len(), 2);
    }
}
struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}
fn create(store: &Store) {
    store
        .create_session(
            "session",
            json!({"type":"session","version":3,"id":"session","cwd":"/fixture","timestamp":NOW})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    store
        .register_origin("origin", "session", &[4; 32])
        .unwrap();
}
fn artifacts<'a>(store: &'a Store, operation: &str) -> ContextStore<'a> {
    ContextStore::new(
        store,
        SessionId::new("session").unwrap(),
        "origin",
        operation,
        NOW,
        Features::default(),
    )
    .unwrap()
}
fn registry() -> Registry {
    let mut r = Registry::builtins().unwrap();
    let names = ["write".into(), "read".into()];
    r.activate(&names, &names.iter().cloned().collect())
        .unwrap();
    r
}
fn scope() -> Scope {
    Scope {
        workspace: "workspace".into(),
        session: "session".into(),
        epoch: 1,
        revision: 0,
    }
}
fn ticket(registry: &Registry, scope: &Scope, operation: &str) -> AuthorizedCall {
    let call = registry
        .prepare(
            operation,
            "call",
            "write",
            &json!({"path":"file","content":"new"}),
            true,
        )
        .unwrap();
    let policy: Policy =
        serde_json::from_value(json!({"mode":"disabled","cwd":"/fixture"})).unwrap();
    let Preflight::Ready(t) = Authorizer::new("incarnation")
        .unwrap()
        .preflight(
            call,
            PreflightContext {
                scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: "target",
                now: 10,
                ttl: 100,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    t
}
fn execution<'a>(registry: &'a Registry, scope: &'a Scope, clock: &'a TestClock) -> Execution<'a> {
    Execution {
        registry,
        fence: ExecutionFence {
            scope,
            policy_revision: 1,
            target_revision: "target",
            now: 10,
        },
        budget: ResultBudget::default(),
        clock,
    }
}
#[test]
fn durable_execution_replays_without_duplicate_effects_or_end_events_after_restart() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("store");
    let store = Store::open(&path).unwrap();
    create(&store);
    let r = registry();
    let scope = scope();
    let clock = TestClock(AtomicU64::new(10));
    let count = AtomicUsize::new(0);
    let mut events = vec![];
    let native = json!({"content":[{"type":"text","text":"first β🦙".repeat(20000)},{"type":"text","text":"second preserved"}],"details":{"opaque":{"keep":true}}});
    let result = executor::execute(
        &store,
        &mut artifacts(&store, "op"),
        ticket(&r, &scope, "op"),
        execution(&r, &scope, &clock),
        &mut NoHooks,
        |t| {
            t.consume(
                &r,
                ExecutionFence {
                    scope: &scope,
                    policy_revision: 1,
                    target_revision: "target",
                    now: 10,
                },
            )?;
            count.fetch_add(1, Ordering::SeqCst);
            clock.0.store(30, Ordering::SeqCst);
            Ok(AdapterOutcome {
                result: native.clone(),
                effect: Effect::Committed,
                artifact_page: false,
            })
        },
        |e| events.push(e),
    )
    .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        events.iter().map(|e| e.stage.as_str()).collect::<Vec<_>>(),
        ["start", "end"]
    );
    assert_ne!(events[0].id, events[1].id);
    assert_eq!(result.record.started_ms, 10);
    assert_eq!(result.record.finished_ms, Some(30));
    assert_eq!(result.record.effect, Effect::Committed);
    assert!(!result.replayed);
    assert_eq!(
        artifacts(&store, "read")
            .read_json(result.record.native_result.as_ref().unwrap())
            .unwrap(),
        native
    );
    let expected = result.result;
    drop(store);
    let store = Store::open(&path).unwrap();
    let replay = executor::execute(
        &store,
        &mut artifacts(&store, "op"),
        ticket(&r, &scope, "op"),
        execution(&r, &scope, &clock),
        &mut NoHooks,
        |_| {
            count.fetch_add(1, Ordering::SeqCst);
            panic!("replayed side effect")
        },
        |_| panic!("duplicate event"),
    )
    .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.result, expected);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let record = executor::recover(&store, "workspace", "session", "op")
        .unwrap()
        .unwrap();
    assert_eq!(record.status, Status::Completed);
    assert!(
        executor::recover(&store, "other", "session", "op")
            .unwrap()
            .is_none()
    );
    let mut changed = scope.clone();
    changed.epoch += 1;
    assert!(matches!(
        executor::execute(
            &store,
            &mut artifacts(&store, "op"),
            ticket(&r, &changed, "op"),
            execution(&r, &changed, &clock),
            &mut NoHooks,
            |_| panic!(),
            |_| panic!()
        ),
        Err(DomainError::Conflict)
    ));
}
#[test]
fn crashed_or_cancelled_adapter_retains_uncertain_intent_and_never_reexecutes() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("store");
    let store = Store::open(&path).unwrap();
    create(&store);
    let r = registry();
    let scope = scope();
    let clock = TestClock(AtomicU64::new(10));
    let count = AtomicUsize::new(0);
    let mut events = vec![];
    let error = executor::execute(
        &store,
        &mut artifacts(&store, "uncertain"),
        ticket(&r, &scope, "uncertain"),
        execution(&r, &scope, &clock),
        &mut NoHooks,
        |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Err(DomainError::Cancelled)
        },
        |e| events.push(e),
    );
    assert!(matches!(error, Err(DomainError::Cancelled)));
    assert_eq!(events.len(), 1);
    drop(store);
    let store = Store::open(&path).unwrap();
    let record = executor::recover(&store, "workspace", "session", "uncertain")
        .unwrap()
        .unwrap();
    assert_eq!(record.status, Status::Intent);
    assert_eq!(record.effect, Effect::Unknown);
    assert!(matches!(
        executor::execute(
            &store,
            &mut artifacts(&store, "uncertain"),
            ticket(&r, &scope, "uncertain"),
            execution(&r, &scope, &clock),
            &mut NoHooks,
            |_| {
                count.fetch_add(1, Ordering::SeqCst);
                panic!()
            },
            |_| panic!()
        ),
        Err(DomainError::DurabilityUnknown)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
struct ReplacingHook;
impl Hooks for ReplacingHook {
    fn after(
        &mut self,
        _: &ag1_core::tools::registry::ValidatedCall,
        _: &Value,
    ) -> Result<Option<Value>, DomainError> {
        Ok(Some(
            json!({"content":[{"type":"text","text":"extension presentation"}],"details":{"inspectionEvidence":{"forged":true},"verification":"passed"},"opaque":{"retained":true}}),
        ))
    }
}
struct BrokenHook;
impl Hooks for BrokenHook {
    fn after(
        &mut self,
        _: &ag1_core::tools::registry::ValidatedCall,
        _: &Value,
    ) -> Result<Option<Value>, DomainError> {
        Err(DomainError::Unavailable("host died".into()))
    }
}
#[test]
fn post_hooks_cannot_forge_native_evidence_or_erase_the_native_result() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path()).unwrap();
    create(&store);
    let r = registry();
    let scope = scope();
    let clock = TestClock(AtomicU64::new(10));
    let native = json!({"content":[{"type":"text","text":"native result"}],"details":{"inspectionEvidence":{"source":"real"}}});
    let result = executor::execute(
        &store,
        &mut artifacts(&store, "custom"),
        ticket(&r, &scope, "custom"),
        execution(&r, &scope, &clock),
        &mut ReplacingHook,
        |_| {
            Ok(AdapterOutcome {
                result: native.clone(),
                effect: Effect::None,
                artifact_page: false,
            })
        },
        |_| {},
    )
    .unwrap();
    assert!(result.result["details"].get("inspectionEvidence").is_none());
    assert_eq!(
        result.result["details"]["extensionDetails"]["inspectionEvidence"]["forged"],
        true
    );
    assert_eq!(
        artifacts(&store, "read")
            .read_json(result.record.native_result.as_ref().unwrap())
            .unwrap(),
        native
    );
    let result = executor::execute(
        &store,
        &mut artifacts(&store, "broken"),
        ticket(&r, &scope, "broken"),
        execution(&r, &scope, &clock),
        &mut BrokenHook,
        |_| {
            Ok(AdapterOutcome {
                result: native.clone(),
                effect: Effect::Committed,
                artifact_page: false,
            })
        },
        |_| {},
    )
    .unwrap();
    assert_eq!(result.result, native);
    assert_eq!(result.record.status, Status::Completed);
    assert!(result.record.post_hook_error.is_some());
}
#[test]
fn aggregate_projection_retains_container_images_and_authorized_utf8_pages_across_reopen() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("store");
    let store = Store::open(&path).unwrap();
    create(&store);
    let raw = json!({"content":[{"type":"text","text":"β🦙".repeat(50000)},{"type":"image","mimeType":"image/png","data":"AA=="},{"type":"text","text":"final text"}],"details":{"unknown":[1,{"keep":"verbatim"}]}});
    let projected = result::project(
        &mut artifacts(&store, "project"),
        "bash",
        "call",
        &raw,
        ResultBudget {
            text_units: 128,
            ..Default::default()
        },
        false,
    )
    .unwrap();
    assert!(projected.projected_text_units <= 128);
    let original = projected.original;
    let text = projected.text_artifacts.first().unwrap().clone();
    let cursor =
        projected.result["details"]["toolResultProjection"]["artifacts"][0]["nextCall"]["cursor"]
            .as_str()
            .unwrap()
            .to_string();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(artifacts(&store, "read").read_json(&original).unwrap(), raw);
    assert_eq!(artifacts(&store, "read").get(&text.id).unwrap(), text);
    let page = store.read_cursor("session", &cursor).unwrap();
    assert!(std::str::from_utf8(&page.bytes).is_ok());
    store
        .create_session(
            "foreign",
            json!({"type":"session","version":3,"id":"foreign","cwd":"/other","timestamp":NOW})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    assert!(store.read_cursor("foreign", &cursor).is_err());
}

struct NthFault {
    point: Boundary,
    remaining: AtomicUsize,
}
impl FaultInjector for NthFault {
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
#[test]
fn intent_and_result_commit_ack_failures_never_repeat_an_effect_or_emit_premature_end() {
    for point in [
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        for nth in [2, 6] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("store");
            let fault = Arc::new(NthFault {
                point,
                remaining: AtomicUsize::new(0),
            });
            let clock = Arc::new(TestClock(AtomicU64::new(10)));
            let store = Store::open_with(&path, clock.clone(), fault.clone()).unwrap();
            create(&store);
            let r = registry();
            let scope = scope();
            let effects = AtomicUsize::new(0);
            let mut events = vec![];
            fault.remaining.store(nth, Ordering::SeqCst);
            let result = executor::execute(
                &store,
                &mut artifacts(&store, "fault"),
                ticket(&r, &scope, "fault"),
                execution(&r, &scope, &clock),
                &mut NoHooks,
                |_| {
                    effects.fetch_add(1, Ordering::SeqCst);
                    Ok(AdapterOutcome {
                        result: json!({"content":[{"type":"text","text":"effect finished"}]}),
                        effect: Effect::Committed,
                        artifact_page: false,
                    })
                },
                |e| events.push(e),
            );
            assert!(result.is_err(), "{point:?} {nth}");
            assert!(events.iter().all(|e| e.stage != "end"));
            assert_eq!(effects.load(Ordering::SeqCst), usize::from(nth == 6));
            drop(store);
            let store = Store::open(&path).unwrap();
            let record = executor::recover(&store, "workspace", "session", "fault").unwrap();
            if point == Boundary::BeforeCommit && nth == 2 {
                assert!(record.is_none());
                continue;
            }
            let record = record.unwrap();
            let completed = nth == 6 && point != Boundary::BeforeCommit;
            assert_eq!(
                record.status,
                if completed {
                    Status::Completed
                } else {
                    Status::Intent
                }
            );
            let replay = executor::execute(
                &store,
                &mut artifacts(&store, "fault"),
                ticket(&r, &scope, "fault"),
                execution(&r, &scope, &clock),
                &mut NoHooks,
                |_| panic!("repeat after {point:?}/{nth}"),
                |_| panic!("duplicate lifecycle event"),
            );
            if completed {
                assert!(replay.unwrap().replayed);
            } else {
                assert!(matches!(replay, Err(DomainError::DurabilityUnknown)));
            }
            assert_eq!(
                store
                    .journal_snapshot(&SessionId::new("session").unwrap())
                    .unwrap()
                    .revision,
                0
            );
        }
    }
}

#[test]
fn malformed_and_policy_rejected_calls_produce_one_durable_error_without_starting_execution() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path()).unwrap();
    create(&store);
    let r = registry();
    let scope = scope();
    let clock = TestClock(AtomicU64::new(10));
    for (operation, name, args, complete) in [
        ("schema", "write", json!({"path":"file","content":[]}), true),
        ("unknown", "missing", json!({}), true),
        (
            "truncated",
            "write",
            json!({"path":"file","content":"part"}),
            false,
        ),
    ] {
        let reason = r
            .prepare(operation, "call", name, &args, complete)
            .unwrap_err();
        let mut events = vec![];
        let rejected = executor::reject(
            &store,
            &mut artifacts(&store, operation),
            executor::Invocation {
                operation,
                call_id: "call",
                name,
                arguments: &args,
                complete,
            },
            executor::Rejection {
                scope: &scope,
                reason: &reason,
                clock: &clock,
                budget: ResultBudget::default(),
            },
            |e| events.push(e),
        )
        .unwrap();
        assert_eq!(rejected.record.status, Status::Rejected);
        assert_eq!(rejected.record.effect, Effect::None);
        assert_eq!(rejected.result["isError"], true);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].stage, "end");
        let again = executor::reject(
            &store,
            &mut artifacts(&store, operation),
            executor::Invocation {
                operation,
                call_id: "call",
                name,
                arguments: &args,
                complete,
            },
            executor::Rejection {
                scope: &scope,
                reason: &reason,
                clock: &clock,
                budget: ResultBudget::default(),
            },
            |_| panic!("duplicate rejected result"),
        )
        .unwrap();
        assert!(again.replayed);
        assert_eq!(again.result, rejected.result);
    }
    let args = json!({"path":"file","content":"blocked"});
    let call = r.prepare("planning", "call", "write", &args, true).unwrap();
    let policy: Policy =
        serde_json::from_value(json!({"mode":"planning","cwd":"/fixture"})).unwrap();
    assert!(matches!(
        Authorizer::new("planner")
            .unwrap()
            .preflight(
                call,
                PreflightContext {
                    scope: &scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: "target",
                    now: 10,
                    ttl: 100,
                    untrusted_extensions: false
                }
            )
            .unwrap(),
        Preflight::Rejected(_)
    ));
    let result = executor::reject(
        &store,
        &mut artifacts(&store, "planning"),
        executor::Invocation {
            operation: "planning",
            call_id: "call",
            name: "write",
            arguments: &args,
            complete: true,
        },
        executor::Rejection {
            scope: &scope,
            reason: &DomainError::PermissionDenied,
            clock: &clock,
            budget: ResultBudget::default(),
        },
        |_| {},
    )
    .unwrap();
    assert_eq!(result.record.effect, Effect::None);
    assert_eq!(
        store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap()
            .revision,
        0
    );
}

#[test]
fn invalid_post_hook_output_settles_known_native_effect_and_replays_without_execution() {
    struct InvalidHook(Value);
    impl Hooks for InvalidHook {
        fn after(
            &mut self,
            _: &ag1_core::tools::registry::ValidatedCall,
            _: &Value,
        ) -> Result<Option<Value>, DomainError> {
            Ok(Some(self.0.clone()))
        }
    }
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path()).unwrap();
    create(&store);
    let r = registry();
    let scope = scope();
    let clock = TestClock(AtomicU64::new(10));
    let native = json!({"content":[{"type":"text","text":"file published"}]});
    let mut deep = json!(null);
    for _ in 0..70 {
        deep = json!({"nested":deep});
    }
    let cases = [
        json!({"content":null}),
        json!({"content":[{"type":"text","text":45}]}),
        json!({"content":[{"type":"unexpected"}]}),
        json!({"content":[{"type":"image","data":"AA==","mimeType":"application/json"}]}),
        deep,
    ];
    for (i, custom) in cases.into_iter().enumerate() {
        let id = format!("invalid-hook-{i}");
        let mut events = vec![];
        let result = executor::execute(
            &store,
            &mut artifacts(&store, &id),
            ticket(&r, &scope, &id),
            execution(&r, &scope, &clock),
            &mut InvalidHook(custom),
            |_| {
                Ok(AdapterOutcome {
                    result: native.clone(),
                    effect: Effect::Committed,
                    artifact_page: false,
                })
            },
            |e| events.push(e),
        )
        .unwrap();
        assert_eq!(result.record.status, Status::Completed);
        assert_eq!(result.record.effect, Effect::Committed);
        assert_eq!(result.result, native);
        assert!(result.record.post_hook_error.is_some());
        assert_eq!(events.len(), 2);
        let replay = executor::execute(
            &store,
            &mut artifacts(&store, &id),
            ticket(&r, &scope, &id),
            execution(&r, &scope, &clock),
            &mut NoHooks,
            |_| panic!("completed mutation must not run again"),
            |_| panic!("duplicate lifecycle event"),
        )
        .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.result, native);
    }
}

#[test]
fn producer_artifact_reuse_authenticates_provenance_and_preserves_exact_utf8_pages() {
    use ag1_core::tools::result::ProducerArtifact;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    let store = Store::open(&path).unwrap();
    create(&store);
    let mut port = artifacts(&store, "producer");
    for (i, complete) in [
        format!("HEAD\n{}TAIL", "full\n".repeat(2000)),
        format!("HEAD\n{}\nTAIL", "🧪漢字".repeat(4000)),
    ]
    .into_iter()
    .enumerate()
    {
        let call = format!("call-source-{i}");
        let source = port
            .put_text(&complete, "terminal-output", "bash", &call, &[])
            .unwrap();
        let grant = ProducerArtifact::authorize(&port, &source.id, "bash", &call).unwrap();
        let raw = json!({"content":[{"type":"text","text":"only the producer tail".repeat(100)}],"details":{"fullOutputArtifact":{"id":source.id}}});
        let projected = result::project_with_producer(
            &mut port,
            "bash",
            &call,
            &raw,
            ResultBudget {
                threshold_bytes: Some(100),
                ..Default::default()
            },
            false,
            Some(&grant),
        )
        .unwrap();
        assert_eq!(projected.text_artifacts, vec![source.clone()]);
        let preview = projected.result["content"][0]["text"].as_str().unwrap();
        assert!(preview.contains("HEAD") && preview.contains("TAIL"));
        assert!(!preview.contains("only the producer tail"));
        assert!(!preview.contains('�'));
        assert_eq!(port.read_json(&projected.original).unwrap(), raw);
        let mut offset = 0;
        let mut recovered = String::new();
        loop {
            let page = store
                .page("session", "origin", &source.id, offset, 1003)
                .unwrap();
            recovered.push_str(std::str::from_utf8(&page.bytes).unwrap());
            let Some(next) = page.next_offset else { break };
            offset = next;
        }
        assert_eq!(recovered, complete);
        assert!(ProducerArtifact::authorize(&port, &source.id, "read", &call).is_err());
        assert!(ProducerArtifact::authorize(&port, &source.id, "bash", "different-call").is_err());
        assert!(
            result::project_with_producer(
                &mut port,
                "bash",
                "different-call",
                &raw,
                ResultBudget::default(),
                false,
                Some(&grant)
            )
            .is_err()
        );
        let db = rusqlite::Connection::open_with_flags(
            path.join("storage.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let count: usize = db
            .query_row("SELECT count(*) FROM artifacts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            count,
            (i + 1) * 2,
            "one producer plus one original-result record per case; no truncated text copy"
        );
    }
    let wrong_kind = port
        .put_text("{}", "tool-original-result", "bash", "call", &[])
        .unwrap();
    assert!(ProducerArtifact::authorize(&port, &wrong_kind.id, "bash", "call").is_err());
}

#[test]
fn post_hook_result_cannot_reuse_a_native_producer_grant_or_forge_one_in_details() {
    struct Replace;
    impl Hooks for Replace {
        fn after(
            &mut self,
            _: &ag1_core::tools::registry::ValidatedCall,
            native: &Value,
        ) -> Result<Option<Value>, DomainError> {
            Ok(Some(
                json!({"content":[{"type":"text","text":"extension replacement\n".repeat(2000)}],"details":{"fullOutputArtifact":native["details"]["fullOutputArtifact"]}}),
            ))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    create(&store);
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(&["bash".into()], &["bash".into()].into())
        .unwrap();
    let scope = scope();
    let clock = TestClock(AtomicU64::new(10));
    let call = registry
        .prepare(
            "op",
            "call",
            "bash",
            &json!({"command":"owned fixture"}),
            true,
        )
        .unwrap();
    let policy: Policy =
        serde_json::from_value(json!({"mode":"disabled","cwd":"/fixture"})).unwrap();
    let Preflight::Ready(ticket) = Authorizer::new("incarnation")
        .unwrap()
        .preflight(
            call,
            PreflightContext {
                scope: &scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: "target",
                now: 10,
                ttl: 100,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let mut port = artifacts(&store, "op");
    let outcome=executor::execute_with_artifacts(&store,&mut port,ticket,execution(&registry,&scope,&clock),&mut Replace,|ticket,port| {
        ticket.consume(&registry,ExecutionFence {scope:&scope,policy_revision:1,target_revision:"target",now:10})?;
        let source=port.put_text(&"native complete output\n".repeat(4000),"terminal-output","bash","call",&[])?;
        Ok(AdapterOutcome { result:json!({"content":[{"type":"text","text":"producer tail\n".repeat(3000)}],"details":{"fullOutputArtifact":{"id":source.id}}}),effect:Effect::None,artifact_page:false })
    }, |_|{}).unwrap();
    let native = port
        .read_json(outcome.record.native_result.as_ref().unwrap())
        .unwrap();
    let producer = native["details"]["fullOutputArtifact"]["id"]
        .as_str()
        .unwrap();
    let projected = outcome.result["details"]["toolResultProjection"]["artifacts"][0]["id"]
        .as_str()
        .unwrap();
    assert_ne!(projected, producer);
    assert!(
        outcome.result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("extension replacement")
    );
    assert!(
        !outcome.result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("native complete output")
    );
    let raw = json!({"content":[{"type":"text","text":"untrusted tail\n".repeat(3000)}],"details":{"fullOutputArtifact":{"id":producer}}});
    let ungranted = result::project(
        &mut port,
        "bash",
        "call",
        &raw,
        ResultBudget::default(),
        false,
    )
    .unwrap();
    assert_ne!(ungranted.text_artifacts[0].id, producer);
    assert!(
        ungranted.result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("untrusted tail")
    );
    assert!(
        !ungranted.result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("native complete output")
    );
}
