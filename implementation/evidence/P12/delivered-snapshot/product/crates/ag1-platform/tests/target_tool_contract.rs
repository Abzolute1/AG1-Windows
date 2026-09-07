#![cfg(target_os = "linux")]
use ag1_core::{
    Clock, DomainError, SessionId,
    context::features::Features,
    tools::{
        approval::{AuthorizedCall, Authorizer, ExecutionFence, Preflight, PreflightContext},
        executor::{self, AdapterOutcome, Effect, Execution, ExecutionArtifacts, NoHooks, Status},
        policy::Policy,
        registry::{Registry, Scope},
        result::ResultBudget,
        targets::{Binding, Capability, Request, VisionAccess},
    },
};
use ag1_platform::{
    resource_fs::ScanControl,
    target_tools::{self, Port, Prepared, Service},
};
use ag1_storage::{Store, context::ContextStore};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
struct TestClock;
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        12
    }
}
struct FixturePort {
    binding: Binding,
    available: bool,
    effects: usize,
    marker: PathBuf,
    store: Arc<Store>,
    session: String,
    passed: Option<bool>,
}
struct Plan {
    binding: Binding,
    operation: String,
}
impl Port for FixturePort {
    type Plan = Plan;
    fn prepare(
        &mut self,
        request: &Request,
        control: &ScanControl<'_>,
    ) -> Result<Plan, DomainError> {
        control.check()?;
        assert_eq!(request.name(), "check");
        if !self.available {
            return Err(DomainError::Unavailable(
                "fixture target unavailable".into(),
            ));
        }
        Ok(Plan {
            binding: self.binding.clone(),
            operation: request.arguments()["fixtureOperation"]
                .as_str()
                .unwrap()
                .into(),
        })
    }
    fn binding(&self, plan: &Plan) -> Binding {
        plan.binding.clone()
    }
    fn display(&self, _: &Plan) -> Value {
        json!({"kind":"owned-fixture","command":["fixture-check"]})
    }
    fn revalidate(&mut self, plan: &Plan, control: &ScanControl<'_>) -> Result<(), DomainError> {
        control.check()?;
        if !self.available || plan.binding != self.binding {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    fn execute<A: ExecutionArtifacts>(
        &mut self,
        plan: Plan,
        _: &Request,
        control: &ScanControl<'_>,
        _: &mut A,
    ) -> Result<AdapterOutcome, DomainError> {
        control.check()?;
        let intent = executor::recover(
            self.store.as_ref(),
            &plan.binding.workspace,
            &self.session,
            &plan.operation,
        )?
        .unwrap();
        assert_eq!(
            intent.status,
            Status::Intent,
            "target effect must follow durable intent"
        );
        self.effects += 1;
        fs::write(&self.marker, "owned fixture effect").unwrap();
        Ok(AdapterOutcome {
            result: json!({"content":[{"type":"text","text":"owned target fixture"}],"isError":self.passed==Some(false),"details":{"fixture":true,"passed":self.passed}}),
            effect: Effect::Committed,
            artifact_page: false,
        })
    }
}
struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    scope: Scope,
    registry: Registry,
    service: Service<FixturePort>,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("store")).unwrap());
        store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":dir.path(),"timestamp":"2026-09-05T00:00:00.000Z"}).to_string().as_bytes()).unwrap();
        store
            .register_origin("fixture", "fixture", &[3; 32])
            .unwrap();
        let scope = Scope {
            workspace: "fixture-workspace".into(),
            session: "fixture".into(),
            epoch: 1,
            revision: 1,
        };
        let mut registry = Registry::builtins().unwrap();
        registry
            .activate(&["check".into()], &["check".into()].into())
            .unwrap();
        let service = Service::new(
            scope.clone(),
            FixturePort {
                binding: Binding {
                    workspace: scope.workspace.clone(),
                    target: "fixture-target".into(),
                    revision: "revision-1".into(),
                    capabilities: [Capability::Check].into(),
                    vision: VisionAccess::default(),
                },
                available: true,
                effects: 0,
                marker: dir.path().join("effect"),
                store: store.clone(),
                session: scope.session.clone(),
                passed: None,
            },
        );
        Self {
            dir,
            store,
            scope,
            registry,
            service,
        }
    }
    fn prepare(&mut self, operation: &str, control: &ScanControl<'_>) -> Prepared<FixturePort> {
        let call = self
            .registry
            .prepare(
                operation,
                operation,
                "check",
                &json!({"fixtureOperation":operation}),
                true,
            )
            .unwrap();
        self.service
            .prepare(&self.registry, call, &self.scope, control)
            .unwrap()
    }
    fn authorize(&self, plan: &Prepared<FixturePort>) -> AuthorizedCall {
        let policy: Policy =
            serde_json::from_value(json!({"mode":"interactive","cwd":self.dir.path()})).unwrap();
        let mut authorizer = Authorizer::new("fixture-authorizer").unwrap();
        match authorizer
            .preflight(
                plan.call().clone(),
                PreflightContext {
                    scope: &self.scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: plan.target_revision(),
                    now: 10,
                    ttl: 1000,
                    untrusted_extensions: false,
                },
            )
            .unwrap()
        {
            Preflight::Ready(ticket) => ticket,
            Preflight::Approval(request) => authorizer
                .answer(
                    &request.binding,
                    true,
                    &self.registry,
                    ExecutionFence {
                        scope: &self.scope,
                        policy_revision: 1,
                        target_revision: plan.target_revision(),
                        now: 11,
                    },
                )
                .unwrap()
                .unwrap(),
            Preflight::Rejected(reason) => panic!("{reason}"),
        }
    }
    fn run(
        &mut self,
        plan: Prepared<FixturePort>,
        authorization: AuthorizedCall,
        control: &ScanControl<'_>,
    ) -> Result<executor::Outcome, DomainError> {
        let operation = plan.call().operation().to_owned();
        let revision = plan.target_revision().to_owned();
        let mut artifacts = ContextStore::new(
            self.store.as_ref(),
            SessionId::new("fixture").unwrap(),
            "fixture",
            &operation,
            "2026-09-05T00:00:00.000Z",
            Features::default(),
        )
        .unwrap();
        self.service.execute(
            plan,
            authorization,
            target_tools::Run {
                store: self.store.as_ref(),
                artifacts: &mut artifacts,
                execution: Execution {
                    registry: &self.registry,
                    fence: ExecutionFence {
                        scope: &self.scope,
                        policy_revision: 1,
                        target_revision: &revision,
                        now: 12,
                    },
                    budget: ResultBudget::default(),
                    clock: &TestClock,
                },
                control,
            },
            &mut NoHooks,
            |_| {},
        )
    }
}
#[test]
fn native_target_check_retains_failed_evidence_and_releases_it_only_on_success() {
    use ag1_core::context::ContextArtifactPort;
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let mut f = Fixture::new();
    let mut artifacts = ContextStore::new(
        f.store.as_ref(),
        SessionId::new("fixture").unwrap(),
        "fixture",
        "edit",
        "2026-09-05T00:00:00.000Z",
        Features::default(),
    )
    .unwrap();
    let evidence = artifacts
        .put_text(
            "exact verifier evidence",
            "tool-output",
            "edit",
            "edit-call",
            &[],
        )
        .unwrap();
    f.store
        .lease(
            "fixture",
            "fixture",
            &evidence.id,
            "retain through verification",
            60000,
            true,
        )
        .unwrap();
    drop(artifacts);
    for passed in [false, true] {
        f.service.port_mut().passed = Some(passed);
        let plan = f.prepare(if passed { "passed" } else { "failed" }, &control);
        let ticket = f.authorize(&plan);
        assert_eq!(f.store.leases("fixture").unwrap().len(), 1);
        let outcome = f.run(plan, ticket, &control).unwrap();
        assert_eq!(outcome.result["details"]["passed"], passed);
        assert_eq!(
            f.store.leases("fixture").unwrap().len(),
            usize::from(!passed)
        );
    }
    assert_eq!(f.service.port_mut().effects, 2);
}
#[test]
fn target_execution_requires_intent_and_replays_without_an_available_target() {
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(2),
    };
    let mut f = Fixture::new();
    let plan = f.prepare("operation", &control);
    let replay = f.prepare("operation", &control);
    let ticket = f.authorize(&plan);
    let replay_ticket = f.authorize(&replay);
    assert_eq!(f.service.port_mut().effects, 0);
    assert!(!f.dir.path().join("effect").exists());
    let result = f.run(plan, ticket, &control).unwrap();
    assert!(!result.replayed);
    assert_eq!(result.record.status, Status::Completed);
    assert_eq!(f.service.port_mut().effects, 1);
    f.service.port_mut().available = false;
    let result = f.run(replay, replay_ticket, &control).unwrap();
    assert!(result.replayed);
    assert_eq!(f.service.port_mut().effects, 1);
}
#[test]
fn stale_target_cancel_and_foreign_service_cannot_enter_the_target_adapter() {
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(2),
    };
    for variant in 0..3 {
        let mut f = Fixture::new();
        let plan = f.prepare("operation", &control);
        let ticket = f.authorize(&plan);
        match variant {
            0 => f.service.port_mut().binding.revision = "changed".into(),
            1 => f.service.port_mut().binding.capabilities.clear(),
            _ => {
                let foreign = Fixture::new();
                f.service = foreign.service;
            }
        }
        assert!(f.run(plan, ticket, &control).is_err());
        assert_eq!(f.service.port_mut().effects, 0);
        assert!(!f.dir.path().join("effect").exists());
    }
    let mut f = Fixture::new();
    let plan = f.prepare("cancel", &control);
    let ticket = f.authorize(&plan);
    let stopped = AtomicBool::new(true);
    let stopped = ScanControl {
        cancel: &stopped,
        deadline: control.deadline,
    };
    assert!(f.run(plan, ticket, &stopped).is_err());
    assert_eq!(f.service.port_mut().effects, 0);
}
