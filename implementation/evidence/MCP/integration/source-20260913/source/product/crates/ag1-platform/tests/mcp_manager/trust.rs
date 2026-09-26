use super::*;
use ag1_core::tools::{
    approval::{Authorizer, Preflight, PreflightContext},
    mcp::{
        Kind,
        trust::{Arguments, Effect, HostRisk, Permission, ReviewedPolicy, Rule, TrustMode},
    },
    policy::Policy,
    registry::{Registry, Scope},
};
use ag1_platform::mcp::manager::{SelectedTool, SessionLease};
use serde_json::json;
use std::sync::Arc;

fn rules() -> Vec<Rule> {
    vec![Rule {
        arguments: Arguments::AllArguments,
        effect: Effect::Read,
        permission: Permission::Allow,
    }]
}
fn revision(m: &Manager, id: &str) -> u64 {
    m.records()
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
        .revision
}
fn connect(m: &Manager, id: &str, scope: &CatalogScope) -> String {
    let connection = m
        .connect(
            m.review_connection(id, scope.clone()).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(m, &connection, ConnectionHealth::Ready);
    connection
}
fn select(
    m: &Manager,
    connection: &str,
    scope: &CatalogScope,
    r: &mut Registry,
) -> ag1_platform::mcp::manager::Selection {
    m.select_tools(
        r,
        scope.clone(),
        &[SelectedTool {
            connection_id: connection.into(),
            original_name: "first".into(),
        }],
    )
    .unwrap()
}
fn preflight(r: &Registry, scope: &CatalogScope) -> Preflight {
    let selected = r.mcp_selection().remove(0);
    let wire = selected.identity().wire_name();
    let owner = Scope {
        workspace: match scope {
            CatalogScope::GeneralChat => "general".into(),
            CatalogScope::Workspace(id) => id.clone(),
        },
        session: "session".into(),
        epoch: 1,
        revision: 1,
    };
    let policy: Policy =
        serde_json::from_value(json!({"mode":"disabled","cwd":"/fixture"})).unwrap();
    Authorizer::new("owner")
        .unwrap()
        .preflight(
            r.prepare("op", "call", &wire, &json!({}), true).unwrap(),
            PreflightContext {
                scope: &owner,
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
fn reviewed_grant_survives_reopen_and_reconnect_without_registry_churn_or_peer_authority() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("state");
    let target = CatalogScope::Workspace("project-a".into());
    let id;
    {
        let m = manager(&store);
        id = m
            .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
            .unwrap()
            .remove(0);
        enable(&m, &id);
        let c = connect(&m, &id, &target);
        let mut r = Registry::builtins().unwrap();
        select(&m, &c, &target, &mut r);
        assert!(matches!(preflight(&r, &target), Preflight::Approval(_)));
        let config = m.records().unwrap()[0].configuration_revision();
        let review = m
            .review_trust(&c, revision(&m, &id), Kind::Tool, "first", rules())
            .unwrap();
        assert_eq!(review.display()["serverId"], id);
        assert_eq!(review.display()["hostClassification"], "unknown");
        assert!(
            review.display()["localAccess"]
                .as_str()
                .unwrap()
                .contains("Host-account")
        );
        m.save_trust(review).unwrap();
        m.wait_disconnected(&c).unwrap();
        assert_eq!(m.records().unwrap()[0].configuration_revision(), config);
        assert_eq!(
            m.scope_policy(&id, &target).unwrap().unwrap().grants.len(),
            1
        );
        let c = connect(&m, &id, &target);
        let selection = select(&m, &c, &target, &mut r);
        assert!(matches!(preflight(&r, &target), Preflight::Ready(_)));
        let old_revision = r.revision();
        select(&m, &c, &target, &mut r);
        assert_eq!(r.revision(), old_revision);
        // Equal-revision independently minted policy cannot replace the manager's witness.
        let definition = r.mcp_selection().remove(0);
        let foreign = ReviewedPolicy::new(
            &definition,
            1,
            TrustMode::Restricted,
            HostRisk::Unknown,
            "foreign native caller".into(),
            rules(),
        )
        .unwrap();
        let mut substituted = Registry::builtins().unwrap();
        substituted
            .replace_mcp_selection(&[definition.with_reviewed_policy(foreign).unwrap()])
            .unwrap();
        assert!(selection.validate(&m, &substituted).is_err());
        m.disconnect(&c).unwrap();
        m.wait_disconnected(&c).unwrap();
    }
    let m = manager(&store);
    let c = connect(&m, &id, &target);
    let mut r = Registry::builtins().unwrap();
    select(&m, &c, &target, &mut r);
    assert!(matches!(preflight(&r, &target), Preflight::Ready(_)));
    let other = CatalogScope::Workspace("project-b".into());
    let b = connect(&m, &id, &other);
    let mut rb = Registry::builtins().unwrap();
    select(&m, &b, &other, &mut rb);
    assert!(matches!(preflight(&rb, &other), Preflight::Approval(_)));
    assert!(events(dir.path()).iter().all(|e| e != "tools/call"));
}

#[test]
fn scoped_revoke_retires_old_registry_witness_but_preserves_unrelated_connection() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let id = m
        .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    enable(&m, &id);
    let a = CatalogScope::Workspace("a".into());
    let b = CatalogScope::Workspace("b".into());
    let first = connect(&m, &id, &a);
    m.save_trust(
        m.review_trust(&first, revision(&m, &id), Kind::Tool, "first", rules())
            .unwrap(),
    )
    .unwrap();
    m.wait_disconnected(&first).unwrap();
    let ca = connect(&m, &id, &a);
    let cb = connect(&m, &id, &b);
    let mut ra = Registry::builtins().unwrap();
    let sa = select(&m, &ca, &a, &mut ra);
    let mut rb = Registry::builtins().unwrap();
    let sb = select(&m, &cb, &b, &mut rb);
    let wire = sa.wire_names().remove(0);
    let old = ra.clone();
    let call = old.prepare("op", "call", &wire, &json!({}), true).unwrap();
    m.revoke_trust(&id, revision(&m, &id), &a, &wire).unwrap();
    assert!(old.validate_current(&call).is_err());
    assert!(sa.validate(&m, &ra).is_err());
    sb.validate(&m, &rb).unwrap();
    assert_eq!(m.status(&cb).unwrap().health, ConnectionHealth::Ready);
    assert!(m.scope_policy(&id, &a).unwrap().unwrap().grants.is_empty());
    assert!(m.revoke_trust(&id, revision(&m, &id), &a, &wire).is_err());
}

#[test]
fn disable_writes_is_durable_and_configuration_changes_do_not_inherit_grants() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let m = manager(&state);
    let id = m
        .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    enable(&m, &id);
    let target = CatalogScope::GeneralChat;
    let c = connect(&m, &id, &target);
    let mut r = Registry::builtins().unwrap();
    select(&m, &c, &target, &mut r);
    m.set_trust_mode(&id, revision(&m, &id), &target, TrustMode::DisableWrites)
        .unwrap();
    m.wait_disconnected(&c).unwrap();
    let c = connect(&m, &id, &target);
    select(&m, &c, &target, &mut r);
    assert!(matches!(preflight(&r, &target), Preflight::Rejected(_)));
    m.save_trust(
        m.review_trust(&c, revision(&m, &id), Kind::Tool, "first", rules())
            .unwrap(),
    )
    .unwrap();
    m.wait_disconnected(&c).unwrap();
    let c = connect(&m, &id, &target);
    select(&m, &c, &target, &mut r);
    assert!(matches!(preflight(&r, &target), Preflight::Ready(_)));
    let mut draft = m.records().unwrap()[0].draft.clone();
    draft.name = "changed configuration".into();
    m.edit(&id, revision(&m, &id), draft).unwrap();
    assert!(
        m.scope_policy(&id, &target)
            .unwrap()
            .unwrap()
            .grants
            .is_empty()
    );
    drop(m);
    let reopened = manager(&state);
    let policy = reopened.scope_policy(&id, &target).unwrap().unwrap();
    assert_eq!(policy.mode, TrustMode::DisableWrites);
    assert!(policy.grants.is_empty());
}

#[test]
fn trust_reviews_are_inert_one_owner_and_stale_after_changes_or_session_close() {
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager(&dir.path().join("state")));
    let id = m
        .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    enable(&m, &id);
    let a = SessionLease::new(m.clone(), CatalogScope::GeneralChat).unwrap();
    let b = SessionLease::new(m.clone(), CatalogScope::GeneralChat).unwrap();
    let c = a
        .connect(
            a.review(&id, false).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &c, ConnectionHealth::Ready);
    assert!(
        m.review_trust(&c, revision(&m, &id), Kind::Tool, "first", rules())
            .is_err()
    );
    assert!(
        b.review_trust(&c, revision(&m, &id), Kind::Tool, "first", rules())
            .is_err()
    );
    let review = a
        .review_trust(&c, revision(&m, &id), Kind::Tool, "first", rules())
        .unwrap();
    assert!(b.save_trust(review).is_err());
    assert!(
        m.scope_policy(&id, &CatalogScope::GeneralChat)
            .unwrap()
            .is_none()
    );
    let review = a
        .review_trust(&c, revision(&m, &id), Kind::Tool, "first", rules())
        .unwrap();
    a.set_trust_mode(&id, revision(&m, &id), TrustMode::DisableWrites)
        .unwrap();
    assert!(a.save_trust(review).is_err());
    m.wait_disconnected(&c).unwrap();
    let c = a
        .connect(
            a.review(&id, false).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &c, ConnectionHealth::Ready);
    let review = a
        .review_trust(&c, revision(&m, &id), Kind::Tool, "first", rules())
        .unwrap();
    a.close().unwrap();
    assert!(a.save_trust(review).is_err());
    assert!(
        m.scope_policy(&id, &CatalogScope::GeneralChat)
            .unwrap()
            .unwrap()
            .grants
            .is_empty()
    );
}

#[test]
fn malformed_rules_and_excess_pending_reviews_do_not_change_saved_policy() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let id = m
        .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    enable(&m, &id);
    let c = connect(&m, &id, &CatalogScope::GeneralChat);
    let initial = revision(&m, &id);
    for rules in [
        Vec::new(),
        vec![rules()[0].clone(); 33],
        vec![Rule {
            arguments: Arguments::Equals(BTreeMap::new()),
            effect: Effect::Read,
            permission: Permission::Allow,
        }],
    ] {
        assert!(
            m.review_trust(&c, initial, Kind::Tool, "first", rules)
                .is_err()
        );
    }
    let reviews: Vec<_> = (0..32)
        .map(|_| {
            m.review_trust(&c, initial, Kind::Tool, "first", rules())
                .unwrap()
        })
        .collect();
    assert!(
        m.review_trust(&c, initial, Kind::Tool, "first", rules())
            .is_err()
    );
    drop(reviews);
    assert!(
        m.review_trust(&c, initial, Kind::Tool, "first", rules())
            .is_ok()
    );
    assert_eq!(revision(&m, &id), initial);
    assert!(
        m.scope_policy(&id, &CatalogScope::GeneralChat)
            .unwrap()
            .is_none()
    );
}

#[test]
fn service_rule_review_requires_native_confirmation_and_does_not_disclose_other_scope_rules() {
    use ag1_platform::mcp::service::Service;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager(&dir.path().join("state")));
    let id = m
        .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    enable(&m, &id);
    let service = Service::from_manager(
        m.clone(),
        Path::new(env!("CARGO_BIN_EXE_ag1-process-host")).into(),
    );
    let owner = Scope {
        workspace: "a".into(),
        session: "session-a".into(),
        epoch: 1,
        revision: 1,
    };
    let other = Scope {
        workspace: "b".into(),
        session: "session-b".into(),
        epoch: 1,
        revision: 1,
    };
    let a = service
        .session(CatalogScope::Workspace("a".into()), owner.clone(), None)
        .unwrap();
    let b = service
        .session(CatalogScope::Workspace("b".into()), other.clone(), None)
        .unwrap();
    let review = a
        .dispatch(
            &owner,
            &json!({"action":"review","id":id,"revision":revision(&m,&id),"test":false}),
        )
        .unwrap();
    let result = a
        .dispatch(
            &owner,
            &json!({"action":"confirm","review":review["review"]}),
        )
        .unwrap();
    let c = result["connection"].as_str().unwrap();
    wait(&m, c, ConnectionHealth::Ready);
    let mut initial_rules = rules();
    initial_rules.push(Rule {
        arguments: Arguments::Equals([("largeId".into(), json!(9007199254740993u64))].into()),
        effect: Effect::Unknown,
        permission: Permission::Deny,
    });
    let command = json!({"action":"trust_review","connection":c,"revision":revision(&m,&id),"kind":"tool","key":"first","rules":initial_rules});
    assert!(b.dispatch(&other, &command).is_err());
    let mut forged = command.clone();
    forged["scope"] = json!({"kind":"workspace","workspace":"b"});
    assert!(a.dispatch(&owner, &forged).is_err());
    let review = a.dispatch(&owner, &command).unwrap();
    assert_eq!(review["details"]["identity"]["binding"]["serverId"], id);
    assert!(
        m.scope_policy(&id, &CatalogScope::Workspace("a".into()))
            .unwrap()
            .is_none()
    );
    assert!(
        a.dispatch(
            &owner,
            &json!({"action":"trust_confirm","review":review["review"],"rules":rules()})
        )
        .is_err()
    );
    a.dispatch(
        &owner,
        &json!({"action":"trust_confirm","review":review["review"]}),
    )
    .unwrap();
    assert!(
        a.dispatch(
            &owner,
            &json!({"action":"trust_confirm","review":review["review"]})
        )
        .is_err()
    );
    m.wait_disconnected(c).unwrap();
    let connection_review = a
        .dispatch(
            &owner,
            &json!({"action":"review","id":id,"revision":revision(&m,&id),"test":false}),
        )
        .unwrap();
    let connected = a
        .dispatch(
            &owner,
            &json!({"action":"confirm","review":connection_review["review"]}),
        )
        .unwrap();
    let connection = connected["connection"].as_str().unwrap();
    wait(&m, connection, ConnectionHealth::Ready);
    let append=a.dispatch(&owner,&json!({"action":"trust_review","connection":connection,"revision":revision(&m,&id),"kind":"tool","key":"first","rules":rules(),"append":true})).unwrap();
    assert_eq!(append["details"]["existingRulesRetained"], 2);
    assert_eq!(
        append["details"]["rules"][1]["arguments"]["fields"]["largeId"],
        json!(9007199254740993u64)
    );
    assert!(
        append["details"]["rulesJson"]
            .as_str()
            .unwrap()
            .contains("9007199254740993")
    );
    a.dispatch(
        &owner,
        &json!({"action":"trust_confirm","review":append["review"]}),
    )
    .unwrap();
    let own = a.dispatch(&owner, &json!({"action":"list"})).unwrap();
    let wire = own["servers"][0]["trustPolicy"]["grants"][0]["wireName"]
        .as_str()
        .unwrap();
    assert!(wire.starts_with("mcp_"));
    assert!(own["servers"][0]["record"].get("trust").is_none());
    let foreign = b.dispatch(&other, &json!({"action":"list"})).unwrap();
    assert!(foreign["servers"][0]["trustPolicy"].is_null());
    assert!(foreign["servers"][0]["record"].get("trust").is_none());
    a.dispatch(
        &owner,
        &json!({"action":"trust_revoke","id":id,"revision":revision(&m,&id),"wire_name":wire}),
    )
    .unwrap();
    assert!(
        m.scope_policy(&id, &CatalogScope::Workspace("a".into()))
            .unwrap()
            .unwrap()
            .grants
            .is_empty()
    );
    assert!(events(dir.path()).iter().all(|e| e != "tools/call"));
    a.close().unwrap();
    b.close().unwrap();
}
