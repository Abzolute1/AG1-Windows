use super::*;
use ag1_core::tools::mcp::{
    Kind,
    trust::{Arguments, Effect as ReviewedEffect, Permission, Rule, TrustMode},
};

#[test]
fn persisted_exact_write_grant_uses_shared_durable_executor_and_revocation_blocks_later_send() {
    for transport in ["stdio", "http"] {
        let mut case = Case::new(transport, Revision::Current20260728, "structured");
        let revision = case.manager.records().unwrap()[0].revision;
        let review = case
            .manager
            .review_trust(
                &case.connection,
                revision,
                Kind::Tool,
                "owned",
                vec![Rule {
                    arguments: Arguments::Equals([("value".into(), json!(1))].into()),
                    effect: ReviewedEffect::Write,
                    permission: Permission::Allow,
                }],
            )
            .unwrap();
        case.manager.save_trust(review).unwrap();
        case.manager.wait_disconnected(&case.connection).unwrap();
        case.connection = case
            .manager
            .connect(
                case.manager
                    .review_connection(&case.server, CatalogScope::GeneralChat)
                    .unwrap(),
                if transport == "stdio" {
                    ConnectionConsent::LocalHostAccountExecution
                } else {
                    ConnectionConsent::LoopbackDevelopmentEndpoint
                },
            )
            .unwrap();
        until(|| case.manager.status(&case.connection).unwrap().health == ConnectionHealth::Ready);
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
        let name = case.selection.wire_names().remove(0);
        let target = case.selection.target_revision(&name).unwrap();
        let policy: Policy =
            serde_json::from_value(json!({"mode":"disabled","cwd":"/fixture"})).unwrap();
        let authorize = |operation: &str, args: Value| {
            Authorizer::new(operation)
                .unwrap()
                .preflight(
                    case.selection
                        .prepare(
                            &case.manager,
                            &case.registry,
                            operation,
                            "call",
                            &name,
                            args.to_string().as_bytes(),
                        )
                        .unwrap(),
                    PreflightContext {
                        scope: &case.scope,
                        policy: &policy,
                        policy_revision: 1,
                        target_revision: &target,
                        now: 100,
                        ttl: 1000,
                        untrusted_extensions: false,
                    },
                )
                .unwrap()
        };
        assert!(matches!(
            authorize("wrong-arguments", json!({"value":2})),
            Preflight::Approval(_)
        ));
        assert!(matches!(
            authorize("consequential", json!({"value":1,"text":"delete database"})),
            Preflight::Approval(_)
        ));
        let Preflight::Ready(ticket) = authorize("reviewed-write", json!({"value":1})) else {
            panic!("reviewed write did not apply")
        };
        let Preflight::Ready(later) = authorize("later-write", json!({"value":1})) else {
            panic!("reviewed write did not apply")
        };
        assert!(case.calls().is_empty());
        let outcome = case
            .execute("reviewed-write", ticket, |event| {
                if event.stage == "start" {
                    assert!(case.calls().is_empty());
                    assert_eq!(
                        executor::recover(&case.store, "workspace", "session", "reviewed-write")
                            .unwrap()
                            .unwrap()
                            .status,
                        Status::Intent
                    );
                }
            })
            .unwrap();
        assert_eq!(outcome.record.status, Status::Completed);
        assert_eq!(
            case.original(&outcome)["result"]["structuredContent"],
            json!({"value":7})
        );
        assert_eq!(case.calls().len(), 1);
        assert!(case.dir.path().join("effects").exists());
        let revision = case.manager.records().unwrap()[0].revision;
        case.manager
            .set_trust_mode(
                &case.server,
                revision,
                &CatalogScope::GeneralChat,
                TrustMode::DisableWrites,
            )
            .unwrap();
        assert!(case.execute("later-write", later, |_| {}).is_err());
        assert_eq!(case.calls().len(), 1);
    }
}
