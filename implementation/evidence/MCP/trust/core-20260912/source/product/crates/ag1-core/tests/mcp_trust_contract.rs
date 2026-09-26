use ag1_core::tools::{
    approval::{Authorizer, ExecutionFence, Preflight, PreflightContext},
    mcp::{
        Binding, Scope as McpScope, SelectedDefinition,
        trust::{Arguments, Effect, HostRisk, Permission, ReviewedPolicy, Rule, TrustMode},
    },
    policy::Policy,
    registry::{Registry, Scope},
};
use serde_json::{Value, json};

fn selected() -> SelectedDefinition {
    SelectedDefinition::new(Binding {
        server_id: "26fb3347-af67-4a1d-92ec-a915db39c5d0".into(),
        scope: McpScope::Workspace("project-a".into()), config_revision: 1,
        auth_profile: "public".into(), credential_generation: 1,
        connection_generation: 1, protocol: "2026-07-28".into(),
    }, &json!({"name":"database","description":"Untrusted mixed operation",
        "inputSchema":{"type":"object","properties":{"action":{"type":"string"},"count":{"type":"integer"},"optional":{"type":["null","string"]}},"required":["action"],"additionalProperties":false},
        "annotations":{"readOnlyHint":true},"trusted":true})).unwrap()
}
fn scope() -> Scope {
    Scope {
        workspace: "project-a".into(),
        session: "session".into(),
        epoch: 1,
        revision: 1,
    }
}
fn rule(effect: Effect, permission: Permission) -> Rule {
    Rule {
        arguments: Arguments::Equals([("action".into(), json!("inspect"))].into()),
        effect,
        permission,
    }
}
fn review(
    selected: &SelectedDefinition,
    mode: TrustMode,
    risk: HostRisk,
    rules: Vec<Rule>,
) -> ReviewedPolicy {
    ReviewedPolicy::new(
        selected,
        1,
        mode,
        risk,
        "Native review of the exact capability and argument cases".into(),
        rules,
    )
    .unwrap()
}
fn registry(selected: SelectedDefinition) -> Registry {
    let mut registry = Registry::builtins().unwrap();
    registry.replace_mcp_selection(&[selected]).unwrap();
    registry
}
fn preflight(
    registry: &Registry,
    selected: &SelectedDefinition,
    mode: &str,
    args: Value,
) -> Preflight {
    let wire = selected.identity().wire_name();
    let policy: Policy =
        serde_json::from_value(json!({"mode":mode,"cwd":"/project-a","trustedTools":[wire]}))
            .unwrap();
    Authorizer::new("trust-owner")
        .unwrap()
        .preflight(
            registry.prepare("op", "call", &wire, &args, true).unwrap(),
            PreflightContext {
                scope: &scope(),
                policy: &policy,
                policy_revision: 1,
                target_revision: &selected.identity().target_revision(),
                now: 1,
                ttl: 100,
                untrusted_extensions: false,
            },
        )
        .unwrap()
}

#[test]
fn read_grant_matches_exact_scalar_arguments_and_planning_policy() {
    let s = selected();
    let p = review(
        &s,
        TrustMode::Restricted,
        HostRisk::Unknown,
        vec![rule(Effect::Read, Permission::Allow)],
    );
    let r = registry(s.clone().with_reviewed_policy(p).unwrap());
    for mode in [
        "interactive",
        "disabled",
        "autonomous",
        "planning",
        "headless",
    ] {
        assert!(
            matches!(
                preflight(&r, &s, mode, json!({"action":"inspect"})),
                Preflight::Ready(_)
            ),
            "{mode}"
        );
    }
    for action in ["Inspect", "inspect ", "delete"] {
        assert!(matches!(
            preflight(&r, &s, "disabled", json!({"action":action})),
            Preflight::Approval(_)
        ));
        assert!(matches!(
            preflight(&r, &s, "planning", json!({"action":action})),
            Preflight::Rejected(_)
        ));
    }
    let policy: Policy = serde_json::from_value(
        json!({"mode":"planning","cwd":"/project-a","planningReadOnlyTools":"block"}),
    )
    .unwrap();
    assert!(matches!(
        Authorizer::new("owner")
            .unwrap()
            .preflight(
                r.prepare(
                    "op",
                    "call",
                    &s.identity().wire_name(),
                    &json!({"action":"inspect"}),
                    true
                )
                .unwrap(),
                PreflightContext {
                    scope: &scope(),
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: &s.identity().target_revision(),
                    now: 1,
                    ttl: 100,
                    untrusted_extensions: false
                }
            )
            .unwrap(),
        Preflight::Rejected(_)
    ));
}

#[test]
fn disable_writes_and_execution_modes_are_stronger_than_reviewed_write_grants() {
    for effect in [
        Effect::Read,
        Effect::Write,
        Effect::Unknown,
        Effect::Consequential,
    ] {
        let s = selected();
        let p = review(
            &s,
            TrustMode::DisableWrites,
            HostRisk::Unknown,
            vec![rule(effect, Permission::Allow)],
        );
        let r = registry(s.clone().with_reviewed_policy(p).unwrap());
        let result = preflight(&r, &s, "disabled", json!({"action":"inspect"}));
        assert_eq!(
            matches!(result, Preflight::Ready(_)),
            effect == Effect::Read
        );
        assert_eq!(
            matches!(result, Preflight::Rejected(_)),
            effect != Effect::Read
        );
        assert!(matches!(
            preflight(&r, &s, "disabled", json!({"action":"unknown"})),
            Preflight::Rejected(_)
        ));
    }
    let s = selected();
    let p = review(
        &s,
        TrustMode::Restricted,
        HostRisk::Unknown,
        vec![rule(Effect::Write, Permission::Allow)],
    );
    let r = registry(s.clone().with_reviewed_policy(p).unwrap());
    assert!(matches!(
        preflight(&r, &s, "interactive", json!({"action":"inspect"})),
        Preflight::Ready(_)
    ));
    for mode in ["planning", "headless"] {
        assert!(matches!(
            preflight(&r, &s, mode, json!({"action":"inspect"})),
            Preflight::Rejected(_)
        ));
    }
}

#[test]
fn host_consequence_floor_and_overlapping_rules_cannot_be_downgraded() {
    let s = selected();
    for rules in [
        vec![rule(Effect::Read, Permission::Allow)],
        vec![
            rule(Effect::Read, Permission::Allow),
            rule(Effect::Write, Permission::Allow),
        ],
    ] {
        let p = review(&s, TrustMode::Restricted, HostRisk::Consequential, rules);
        let r = registry(s.clone().with_reviewed_policy(p).unwrap());
        for mode in ["disabled", "interactive", "autonomous"] {
            let Preflight::Approval(request) = preflight(&r, &s, mode, json!({"action":"inspect"}))
            else {
                panic!("consequence bypass")
            };
            assert_eq!(request.display["risk"], "consequential");
            assert_eq!(request.display["hostRisk"], "consequential");
        }
    }
    for (second, expected) in [
        (rule(Effect::Read, Permission::Deny), "deny"),
        (rule(Effect::Read, Permission::Ask), "ask"),
        (rule(Effect::Unknown, Permission::Allow), "ask"),
        (rule(Effect::Consequential, Permission::Allow), "ask"),
    ] {
        for rules in [
            vec![rule(Effect::Read, Permission::Allow), second.clone()],
            vec![second, rule(Effect::Read, Permission::Allow)],
        ] {
            let p = review(&s, TrustMode::Restricted, HostRisk::Unknown, rules);
            let r = registry(s.clone().with_reviewed_policy(p).unwrap());
            let result = preflight(&r, &s, "disabled", json!({"action":"inspect"}));
            assert_eq!(matches!(result, Preflight::Rejected(_)), expected == "deny");
            assert_eq!(matches!(result, Preflight::Approval(_)), expected == "ask");
        }
    }
}

#[test]
fn native_argument_escalation_overrides_a_reviewed_read_and_server_readonly_hint() {
    let s = selected();
    for mode in [TrustMode::Restricted, TrustMode::DisableWrites] {
        let p = review(
            &s,
            mode,
            HostRisk::Unknown,
            vec![Rule {
                arguments: Arguments::AllArguments,
                effect: Effect::Read,
                permission: Permission::Allow,
            }],
        );
        let r = registry(s.clone().with_reviewed_policy(p).unwrap());
        for args in [
            json!({"action":"deleteDatabase"}),
            json!({"action":"inspect","optional":"DELETE FROM issues"}),
            json!({"action":"set_permissions"}),
            json!({"action":"purchase"}),
            json!({"action":"rotateCredentials"}),
            json!({"action":"publish"}),
        ] {
            let result = preflight(&r, &s, "disabled", args.clone());
            if mode == TrustMode::DisableWrites {
                assert!(matches!(result, Preflight::Rejected(_)));
            } else {
                let Preflight::Approval(request) = result else {
                    panic!("native consequence bypass for {args}")
                };
                assert_eq!(request.display["hostRisk"], "consequential");
                assert_eq!(request.display["risk"], "consequential");
                assert!(
                    request.display["classificationBasis"]
                        .as_str()
                        .unwrap()
                        .contains("operation name or argument")
                );
            }
            assert!(matches!(
                preflight(&r, &s, "planning", args),
                Preflight::Rejected(_)
            ));
        }
    }
}

#[test]
fn revocation_invalidates_prepared_calls_and_tickets_even_in_cloned_registries() {
    for permission in [Permission::Allow, Permission::Ask] {
        let s = selected();
        let p = review(
            &s,
            TrustMode::Restricted,
            HostRisk::Unknown,
            vec![rule(Effect::Read, permission)],
        );
        let r = registry(s.clone().with_reviewed_policy(p.clone()).unwrap());
        let old = r.clone();
        let wire = s.identity().wire_name();
        let call = old
            .prepare("op", "call", &wire, &json!({"action":"inspect"}), true)
            .unwrap();
        let owner_scope = scope();
        let target = s.identity().target_revision();
        let policy: Policy =
            serde_json::from_value(json!({"mode":"disabled","cwd":"/project-a"})).unwrap();
        let mut authorizer = Authorizer::new("owner").unwrap();
        let context = || PreflightContext {
            scope: &owner_scope,
            policy: &policy,
            policy_revision: 1,
            target_revision: &target,
            now: 1,
            ttl: 100,
            untrusted_extensions: false,
        };
        let ready = authorizer.preflight(call.clone(), context()).unwrap();
        let fence = ExecutionFence {
            scope: &owner_scope,
            policy_revision: 1,
            target_revision: &target,
            now: 2,
        };
        let ticket = match ready {
            Preflight::Ready(ticket) => ticket,
            Preflight::Approval(request) => authorizer
                .answer(&request.binding, true, &old, fence)
                .unwrap()
                .unwrap(),
            _ => panic!("unexpected denial"),
        };
        ticket.check(&old, fence).unwrap();
        p.revoke();
        assert!(ticket.check(&old, fence).is_err());
        assert!(old.validate_current(&call).is_err());
        assert!(
            old.prepare("new", "call", &wire, &json!({"action":"inspect"}), true)
                .is_err()
        );
        assert!(
            Authorizer::new("other")
                .unwrap()
                .preflight(call, context())
                .is_err()
        );
        let mut copy = old.clone();
        assert!(copy.replace_mcp_selection(&old.mcp_selection()).is_err());
    }
}

#[test]
fn policy_replacement_invalidates_pending_approval_and_equal_revision_foreign_policy() {
    let s = selected();
    let p = review(
        &s,
        TrustMode::Restricted,
        HostRisk::Unknown,
        vec![rule(Effect::Read, Permission::Ask)],
    );
    let mut r = registry(s.clone().with_reviewed_policy(p.clone()).unwrap());
    let call = r
        .prepare(
            "op",
            "call",
            &s.identity().wire_name(),
            &json!({"action":"inspect"}),
            true,
        )
        .unwrap();
    let old_revision = r.revision();
    r.replace_mcp_selection(&r.mcp_selection()).unwrap();
    assert_eq!(old_revision, r.revision());
    let other = review(
        &s,
        TrustMode::Restricted,
        HostRisk::Unknown,
        vec![rule(Effect::Read, Permission::Ask)],
    );
    let foreign = registry(s.clone().with_reviewed_policy(other.clone()).unwrap());
    assert_eq!(old_revision, foreign.revision());
    assert!(foreign.validate_current(&call).is_err());
    let owner_scope = scope();
    let target = s.identity().target_revision();
    let policy: Policy =
        serde_json::from_value(json!({"mode":"interactive","cwd":"/project-a"})).unwrap();
    let mut authorizer = Authorizer::new("owner").unwrap();
    let Preflight::Approval(request) = authorizer
        .preflight(
            call.clone(),
            PreflightContext {
                scope: &owner_scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: &target,
                now: 1,
                ttl: 100,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!("missing approval")
    };
    let context = || PreflightContext {
        scope: &owner_scope,
        policy: &policy,
        policy_revision: 1,
        target_revision: &target,
        now: 2,
        ttl: 100,
        untrusted_extensions: false,
    };
    let foreign_call = foreign
        .prepare(
            "op",
            "call",
            &s.identity().wire_name(),
            &json!({"action":"inspect"}),
            true,
        )
        .unwrap();
    assert!(authorizer.preflight(foreign_call, context()).is_err());
    p.revoke();
    assert!(authorizer.preflight(call.clone(), context()).is_err());
    assert!(
        authorizer
            .answer(
                &request.binding,
                true,
                &r,
                ExecutionFence {
                    scope: &owner_scope,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 2
                }
            )
            .is_err()
    );
    r.replace_mcp_selection(&[s.with_reviewed_policy(other).unwrap()])
        .unwrap();
    assert!(r.revision() > old_revision);
    assert!(r.validate_current(&call).is_err());
}

#[test]
fn reviewed_policy_cannot_transfer_across_any_bound_identity_dimension() {
    let s = selected();
    let p = review(
        &s,
        TrustMode::Restricted,
        HostRisk::Unknown,
        vec![rule(Effect::Read, Permission::Allow)],
    );
    for field in [
        "server",
        "scope",
        "config",
        "profile",
        "credential",
        "connection",
        "protocol",
        "schema",
        "name",
        "kind",
    ] {
        let mut identity = s.identity().clone();
        match field {
            "server" => identity.binding.server_id = "36fb3347-af67-4a1d-92ec-a915db39c5d0".into(),
            "scope" => identity.binding.scope = McpScope::Workspace("project-b".into()),
            "config" => identity.binding.config_revision += 1,
            "profile" => identity.binding.auth_profile = "different".into(),
            "credential" => identity.binding.credential_generation += 1,
            "connection" => identity.binding.connection_generation += 1,
            "protocol" => identity.binding.protocol = "2025-11-25".into(),
            "schema" => identity.metadata_digest = "a".repeat(64),
            "name" => identity.original_name = "another".into(),
            "kind" => identity.kind = ag1_core::tools::mcp::Kind::Prompt,
            _ => unreachable!(),
        }
        assert!(p.check(&identity).is_err(), "{field}");
    }
    let mut b = s.identity().binding.clone();
    b.config_revision += 1;
    let different = SelectedDefinition::new(
        b,
        &json!({"name":"database","inputSchema":{"type":"object"}}),
    )
    .unwrap();
    assert!(different.with_reviewed_policy(p).is_err());
}

#[test]
fn malformed_or_unbounded_rules_are_rejected_and_null_does_not_match_absence() {
    let s = selected();
    for fields in [
        json!({}),
        json!({"action":{}}),
        json!({"action":[]}),
        json!({"action":"x".repeat(4097)}),
    ] {
        let rule: Rule = serde_json::from_value(json!({"arguments":{"kind":"equals","fields":fields},"effect":"read","permission":"allow"})).unwrap();
        assert!(
            ReviewedPolicy::new(
                &s,
                1,
                TrustMode::Restricted,
                HostRisk::Unknown,
                "review".into(),
                vec![rule]
            )
            .is_err()
        );
    }
    assert!(
        ReviewedPolicy::new(
            &s,
            1,
            TrustMode::Restricted,
            HostRisk::Unknown,
            "review".into(),
            vec![rule(Effect::Read, Permission::Allow); 33]
        )
        .is_err()
    );
    assert!(serde_json::from_value::<Rule>(json!({"arguments":{"kind":"allArguments","trusted":true},"effect":"read","permission":"allow"})).is_err());
    let rule = Rule {
        arguments: Arguments::Equals([("optional".into(), Value::Null)].into()),
        effect: Effect::Read,
        permission: Permission::Allow,
    };
    let p = review(&s, TrustMode::Restricted, HostRisk::Unknown, vec![rule]);
    let r = registry(s.clone().with_reviewed_policy(p).unwrap());
    assert!(matches!(
        preflight(&r, &s, "disabled", json!({"action":"inspect"})),
        Preflight::Approval(_)
    ));
    assert!(matches!(
        preflight(
            &r,
            &s,
            "disabled",
            json!({"action":"inspect","optional":null})
        ),
        Preflight::Ready(_)
    ));
}
