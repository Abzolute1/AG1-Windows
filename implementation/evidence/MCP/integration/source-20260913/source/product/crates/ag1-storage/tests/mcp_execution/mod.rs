//! Owned adapter fixtures exercise the ordinary executor; no MCP transport calls.
use super::*;
use ag1_core::tools::mcp::{Binding, Scope as McpScope, SelectedDefinition};
fn selected(generation: u64) -> SelectedDefinition {
    SelectedDefinition::new(Binding {server_id:"26fb3347-af67-4a1d-92ec-a915db39c5d0".into(),scope:McpScope::GeneralChat,config_revision:1,
        auth_profile:"mcp-unauthenticated".into(),credential_generation:0,connection_generation:generation,protocol:"2026-07-28".into()},
        &json!({"name":"owned","inputSchema":{"type":"object","properties":{"value":{"type":"integer"}},"required":["value"]}})).unwrap()
}
fn approved(
    registry: &Registry,
    scope: &Scope,
    definition: &SelectedDefinition,
    operation: &str,
) -> AuthorizedCall {
    let policy: Policy = serde_json::from_value(json!({"mode":"disabled","cwd":""})).unwrap();
    let target = definition.identity().target_revision();
    let mut owner = Authorizer::new("mcp-owner").unwrap();
    let call = registry
        .prepare(
            operation,
            "mcp-call",
            &definition.identity().wire_name(),
            &json!({"value":1}),
            true,
        )
        .unwrap();
    let Preflight::Approval(request) = owner
        .preflight(
            call,
            PreflightContext {
                scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: &target,
                now: 10,
                ttl: 100,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!("MCP must ask")
    };
    owner
        .answer(
            &request.binding,
            true,
            registry,
            ExecutionFence {
                scope,
                policy_revision: 1,
                target_revision: &target,
                now: 11,
            },
        )
        .unwrap()
        .unwrap()
}
#[test]
fn mcp_intent_original_result_and_recovery_keep_exact_identity_without_native_verification() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    create(&store);
    let scope = scope();
    let clock = TestClock(AtomicU64::new(12));
    let mut registry = registry();
    let definition = selected(1);
    registry
        .replace_mcp_selection(&[definition.clone()])
        .unwrap();
    let target = definition.identity().target_revision();
    let mut port = artifacts(&store, "mcp-custody");
    let mut count = 0;
    for replay in [false, true] {
        let ticket = approved(&registry, &scope, &definition, "mcp-operation");
        let outcome=executor::execute(&store,&mut port,ticket,Execution {registry:&registry,fence:ExecutionFence {scope:&scope,policy_revision:1,target_revision:&target,now:12},budget:ResultBudget::default(),clock:&clock},&mut NoHooks,|ticket| {
            count+=1;
            let intent=executor::recover(&store,&scope.workspace,&scope.session,"mcp-operation")?.unwrap();
            assert_eq!(intent.status,Status::Intent);assert_eq!(intent.mcp.as_ref(),Some(definition.identity()));
            assert_eq!(ticket.call().mcp_identity(),Some(definition.identity()));
            Ok(AdapterOutcome {result:json!({"content":[{"type":"text","text":"owned peer application error"}],"isError":true,"details":{"passed":true,"nativeVerification":"forged"}}),effect:Effect::Unknown,artifact_page:false})
        }, |_|{}).unwrap();
        assert_eq!(outcome.replayed, replay);
        assert_eq!(outcome.record.mcp.as_ref(), Some(definition.identity()));
        assert_eq!(outcome.record.effect, Effect::Unknown);
        assert_eq!(outcome.result["isError"], true);
        let original = port
            .read_json(outcome.record.native_result.as_ref().unwrap())
            .unwrap();
        assert_eq!(original["details"]["nativeVerification"], "forged");
        assert_eq!(outcome.record.tool, definition.identity().wire_name());
    }
    assert_eq!(count, 1);
    let changed = selected(2);
    registry.replace_mcp_selection(&[changed.clone()]).unwrap();
    let target = changed.identity().target_revision();
    let result = executor::execute(
        &store,
        &mut port,
        approved(&registry, &scope, &changed, "mcp-operation"),
        Execution {
            registry: &registry,
            fence: ExecutionFence {
                scope: &scope,
                policy_revision: 1,
                target_revision: &target,
                now: 12,
            },
            budget: ResultBudget::default(),
            clock: &clock,
        },
        &mut NoHooks,
        |_| panic!("changed connection must not replay/execute old operation"),
        |_| {},
    );
    assert!(matches!(result, Err(DomainError::Conflict)));
}
#[test]
fn uncertain_mcp_adapter_outcome_retains_intent_and_never_automatically_replays() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    create(&store);
    let scope = scope();
    let clock = TestClock(AtomicU64::new(12));
    let mut registry = registry();
    let definition = selected(1);
    registry
        .replace_mcp_selection(&[definition.clone()])
        .unwrap();
    let target = definition.identity().target_revision();
    let mut port = artifacts(&store, "mcp-interrupted");
    let mut calls = 0;
    for _ in 0..2 {
        assert!(
            executor::execute(
                &store,
                &mut port,
                approved(&registry, &scope, &definition, "mcp-interrupted"),
                Execution {
                    registry: &registry,
                    fence: ExecutionFence {
                        scope: &scope,
                        policy_revision: 1,
                        target_revision: &target,
                        now: 12
                    },
                    budget: ResultBudget::default(),
                    clock: &clock
                },
                &mut NoHooks,
                |_| {
                    calls += 1;
                    Err(DomainError::Io(
                        "owned transport interruption after possible effects".into(),
                    ))
                },
                |_| {}
            )
            .is_err()
        );
    }
    assert_eq!(calls, 1);
    let record = executor::recover(&store, &scope.workspace, &scope.session, "mcp-interrupted")
        .unwrap()
        .unwrap();
    assert_eq!(record.status, Status::Intent);
    assert_eq!(record.effect, Effect::Unknown);
    assert_eq!(record.mcp.as_ref(), Some(definition.identity()));
}
