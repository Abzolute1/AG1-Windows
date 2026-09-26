use super::*;
use ag1_platform::mcp::{
    manager::{ConnectionConsent, ConnectionHealth, SessionLease},
    service::Service,
};
fn owner(f: &Fixture) -> Scope {
    Scope {
        workspace: f.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: 0,
    }
}
fn stop_legacy(peer: &super::super::Case) {
    peer.manager.disconnect(&peer.connection).unwrap();
    peer.manager.wait_disconnected(&peer.connection).unwrap();
}
fn ready(peer: &super::super::Case, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while peer.manager.status(id).unwrap().health != ConnectionHealth::Ready {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn connect(
    session: &ag1_platform::mcp::service::Session,
    scope: &Scope,
    peer: &super::super::Case,
) -> String {
    let revision = peer.manager.records().unwrap()[0].revision;
    let review = session
        .dispatch(
            scope,
            &json!({"action":"review","id":peer.server,"revision":revision,"test":false}),
        )
        .unwrap();
    let connected = session
        .dispatch(
            scope,
            &json!({"action":"confirm","review":review["review"]}),
        )
        .unwrap();
    let id = connected["connection"].as_str().unwrap().to_owned();
    ready(peer, &id);
    id
}
#[test]
fn same_scope_session_leases_cannot_discover_select_inspect_or_close_each_other() {
    let peer = super::super::Case::new("stdio", Revision::Current20260728, "structured");
    stop_legacy(&peer);
    let a = SessionLease::new(peer.manager.clone(), CatalogScope::GeneralChat).unwrap();
    let b = SessionLease::new(peer.manager.clone(), CatalogScope::GeneralChat).unwrap();
    let foreign_review = a.review(&peer.server, false).unwrap();
    assert!(
        b.connect(foreign_review, ConnectionConsent::LocalHostAccountExecution)
            .is_err()
    );
    let ca = a
        .connect(
            a.review(&peer.server, false).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    let cb = b
        .connect(
            b.review(&peer.server, false).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    ready(&peer, &ca);
    ready(&peer, &cb);
    assert!(
        a.connect(
            a.review(&peer.server, false).unwrap(),
            ConnectionConsent::LocalHostAccountExecution
        )
        .is_err()
    );
    assert_eq!(
        a.search(Kind::Tool, "", 20).unwrap().matches[0].connection_id,
        ca
    );
    assert_eq!(
        b.search(Kind::Tool, "", 20).unwrap().matches[0].connection_id,
        cb
    );
    assert!(
        peer.manager
            .search_tools(&CatalogScope::GeneralChat, "", 20)
            .unwrap()
            .matches
            .is_empty()
    );
    assert!(
        peer.manager
            .search_capabilities(&CatalogScope::GeneralChat, Kind::Tool, "", 20)
            .unwrap()
            .matches
            .is_empty()
    );
    assert!(b.inspect(&ca).is_err());
    assert!(b.disconnect(&ca).is_err());
    assert!(
        b.select(
            &mut Registry::builtins().unwrap(),
            &[SelectedCapability {
                connection_id: ca.clone(),
                kind: Kind::Tool,
                key: "owned".into()
            }]
        )
        .is_err()
    );
    a.close().unwrap();
    assert!(a.search(Kind::Tool, "", 20).is_err());
    assert_eq!(
        peer.manager.status(&cb).unwrap().health,
        ConnectionHealth::Ready
    );
    b.inspect(&cb).unwrap();
    b.close().unwrap();
}
#[test]
fn denying_one_workspace_preserves_other_lease_catalog_and_selected_identity() {
    let peer = super::super::Case::new("http", Revision::Current20260728, "structured");
    stop_legacy(&peer);
    let mut record = peer.manager.records().unwrap().remove(0);
    record.availability.all_workspaces = true;
    peer.manager
        .set_availability(&peer.server, record.revision, record.availability)
        .unwrap();
    let a = SessionLease::new(peer.manager.clone(), CatalogScope::Workspace("a".into())).unwrap();
    let b = SessionLease::new(peer.manager.clone(), CatalogScope::Workspace("b".into())).unwrap();
    let ca = a
        .connect(
            a.review(&peer.server, false).unwrap(),
            ConnectionConsent::LoopbackDevelopmentEndpoint,
        )
        .unwrap();
    let cb = b
        .connect(
            b.review(&peer.server, false).unwrap(),
            ConnectionConsent::LoopbackDevelopmentEndpoint,
        )
        .unwrap();
    ready(&peer, &ca);
    ready(&peer, &cb);
    let mut registry = Registry::builtins().unwrap();
    let selection = b
        .select(
            &mut registry,
            &[SelectedCapability {
                connection_id: cb.clone(),
                kind: Kind::Tool,
                key: "owned".into(),
            }],
        )
        .unwrap();
    let record = peer.manager.records().unwrap().remove(0);
    let config = record.configuration_revision();
    let mut availability = record.availability;
    availability.denied_workspaces.insert("a".into());
    peer.manager
        .set_availability(&peer.server, record.revision, availability)
        .unwrap();
    assert!(a.inspect(&ca).is_err());
    assert_eq!(
        peer.manager.status(&cb).unwrap().health,
        ConnectionHealth::Ready
    );
    assert_eq!(
        peer.manager.records().unwrap()[0].configuration_revision(),
        config
    );
    selection.validate(&peer.manager, &registry).unwrap();
    b.inspect(&cb).unwrap();
    a.close().unwrap();
    b.close().unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manager_owned_connection_runs_in_native_loop_after_other_chat_closes() {
    let peer = super::super::Case::new("stdio", Revision::Current20260728, "structured");
    stop_legacy(&peer);
    let name = peer.selection.wire_names().remove(0);
    let (origin, server) = server(vec![
        calls(vec![("owned-call", &name, json!({"value":7}))]),
        stop(),
    ]);
    let mut f = fixture(&origin, "interactive");
    let scope = owner(&f);
    let service = Service::from_manager(peer.manager.clone(), std::path::PathBuf::new());
    let session = service
        .session(CatalogScope::GeneralChat, scope.clone(), None)
        .unwrap();
    let other_scope = Scope {
        session: "other".into(),
        ..scope.clone()
    };
    let other = service
        .session(CatalogScope::GeneralChat, other_scope.clone(), None)
        .unwrap();
    let connection = connect(&session, &scope, &peer);
    connect(&other, &other_scope, &peer);
    session
        .discovery()
        .select(
            &scope,
            &[SelectedCapability {
                connection_id: connection,
                kind: Kind::Tool,
                key: "owned".into(),
            }],
        )
        .unwrap();
    let mut runner = f.files.as_ref().clone();
    let mut registry = runner.registry.as_ref().clone();
    session.discovery().register(&mut registry).unwrap();
    runner.registry = Arc::new(registry);
    runner.adapters.mcp_discovery = Some(session.discovery().clone());
    runner.adapters.mcp_management = Some(session.clone());
    let approval = Arc::new(Accept(AtomicUsize::new(0)));
    runner.approvals = approval.clone();
    f.files = Arc::new(runner);
    other.close().unwrap();
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(approval.0.load(Ordering::SeqCst), 1);
    assert_eq!(messages(&f)[0]["message"]["isError"], false);
    let requests = server.join().unwrap();
    assert!(
        requests[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["function"]["name"] == "mcpDiscover"),
        "existing saved selection gains the native facade only for the request"
    );
    session.close().unwrap();
}
#[test]
fn management_review_is_inert_scope_bound_consumed_once_and_configuration_cas_checked() {
    let peer = super::super::Case::new("stdio", Revision::Current20260728, "structured");
    stop_legacy(&peer);
    let service = Service::from_manager(peer.manager.clone(), std::path::PathBuf::new());
    let scope = Scope {
        workspace: "owner".into(),
        session: "session".into(),
        epoch: 1,
        revision: 0,
    };
    let session = service
        .session(CatalogScope::GeneralChat, scope.clone(), None)
        .unwrap();
    let record = peer.manager.records().unwrap().remove(0);
    let before = std::fs::read(peer.dir.path().join("events.jsonl")).unwrap();
    let review = session
        .dispatch(
            &scope,
            &json!({"action":"review","id":peer.server,"revision":record.revision,"test":false}),
        )
        .unwrap();
    assert_eq!(
        std::fs::read(peer.dir.path().join("events.jsonl")).unwrap(),
        before
    );
    assert!(
        session
            .dispatch(
                &Scope {
                    epoch: 2,
                    ..scope.clone()
                },
                &json!({"action":"confirm","review":review["review"]})
            )
            .is_err()
    );
    assert!(
        session
            .dispatch(
                &scope,
                &json!({"action":"confirm","review":review["review"],"consent":"anything"})
            )
            .is_err()
    );
    session.dispatch(&scope,&json!({"action":"availability","id":peer.server,"revision":record.revision,"enabled":false,"target":"current"})).unwrap();
    assert!(
        session
            .dispatch(
                &scope,
                &json!({"action":"confirm","review":review["review"]})
            )
            .is_err()
    );
    assert!(
        session
            .dispatch(
                &scope,
                &json!({"action":"confirm","review":review["review"]})
            )
            .is_err()
    );
    assert!(session.dispatch(&scope,&json!({"action":"availability","id":peer.server,"revision":record.revision,"enabled":true,"target":"current"})).is_err());
    assert!(!peer.dir.path().join("effects").exists());
    session.close().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_project_capabilities_execute_under_separate_conversation_owner() {
    for (kind, mode, key, args) in [
        (Kind::Tool, "structured", "owned", json!({"value":7})),
        (
            Kind::Resource,
            "content_ok",
            "fixture://owned/source",
            json!({}),
        ),
        (
            Kind::ResourceTemplate,
            "content_ok",
            "fixture://owned/{path}?q={query}",
            json!({"path":"source","query":"literal"}),
        ),
        (
            Kind::Prompt,
            "content_ok",
            "owned",
            json!({"topic":"literal"}),
        ),
    ] {
        let mut peer = super::super::Case::new("stdio", Revision::Current20260728, mode);
        if kind != Kind::Tool {
            peer.content_selection(kind);
        }
        let name = peer.selection.wire_names().remove(0);
        stop_legacy(&peer);
        let mut record = peer.manager.records().unwrap().remove(0);
        record
            .availability
            .workspaces
            .insert("canonical-project".into());
        peer.manager
            .set_availability(&peer.server, record.revision, record.availability)
            .unwrap();
        let (origin, provider) = server(vec![calls(vec![("project-call", &name, args)]), stop()]);
        let mut f = fixture(&origin, "interactive");
        let scope = owner(&f);
        assert_ne!(scope.workspace, "canonical-project");
        let service = Service::from_manager(peer.manager.clone(), Default::default());
        let session = service
            .session(
                CatalogScope::Workspace("canonical-project".into()),
                scope.clone(),
                None,
            )
            .unwrap();
        let connection = connect(&session, &scope, &peer);
        session
            .discovery()
            .select(
                &scope,
                &[SelectedCapability {
                    connection_id: connection,
                    kind,
                    key: key.into(),
                }],
            )
            .unwrap();
        let mut runner = f.files.as_ref().clone();
        let mut registry = runner.registry.as_ref().clone();
        session.discovery().register(&mut registry).unwrap();
        runner.registry = Arc::new(registry);
        runner.adapters.mcp_discovery = Some(session.discovery().clone());
        runner.adapters.mcp_management = Some(session.clone());
        let approval = Arc::new(Accept(AtomicUsize::new(0)));
        runner.approvals = approval.clone();
        f.files = Arc::new(runner);
        f.host
            .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
            .await
            .unwrap();
        assert_eq!(approval.0.load(Ordering::SeqCst), 1, "{kind:?}");
        assert_eq!(messages(&f)[0]["message"]["isError"], false, "{kind:?}");
        assert_eq!(provider.join().unwrap().len(), 2);
        session.close().unwrap();
    }
}

#[test]
fn manager_detail_keeps_full_metadata_and_exact_generation_without_granting_test_execution() {
    let peer = super::super::Case::new("stdio", Revision::Current20260728, "content_ok");
    stop_legacy(&peer);
    let service = Service::from_manager(peer.manager.clone(), Default::default());
    let scope = Scope {
        workspace: "chat-owner".into(),
        session: "chat".into(),
        epoch: 7,
        revision: 0,
    };
    let session = service
        .session(CatalogScope::GeneralChat, scope.clone(), None)
        .unwrap();
    let record = peer.manager.records().unwrap().remove(0);
    let review = session
        .dispatch(
            &scope,
            &json!({"action":"review","id":peer.server,"revision":record.revision,"test":true}),
        )
        .unwrap();
    let connected = session
        .dispatch(
            &scope,
            &json!({"action":"confirm","review":review["review"]}),
        )
        .unwrap();
    let id = connected["connection"].as_str().unwrap();
    ready(&peer, id);
    let catalog = peer
        .manager
        .inspect_catalog(id, &CatalogScope::GeneralChat)
        .unwrap();
    let detail = session
        .dispatch(
            &scope,
            &json!({"action":"detail","connection":id,"kind":"prompt","key":"owned"}),
        )
        .unwrap();
    assert_eq!(
        detail["binding"]["connectionGeneration"],
        catalog.binding.connection_generation.to_string()
    );
    assert_eq!(detail["identity"]["binding"], detail["binding"]);
    assert_eq!(detail["identity"]["executionScope"]["epoch"], "7");
    assert_eq!(detail["schemaSupported"], true);
    assert_eq!(detail["executable"], false);
    assert!(
        detail["unavailableReason"]
            .as_str()
            .unwrap()
            .contains("inspection only")
    );
    assert_eq!(
        serde_json::from_str::<Value>(detail["originalJson"].as_str().unwrap()).unwrap(),
        detail["original"]
    );
    let unsupported=session.dispatch(&scope,&json!({"action":"detail","connection":id,"kind":"resourceTemplate","key":"fixture://owned/{+path}"})).unwrap();
    assert_eq!(unsupported["schemaSupported"], false);
    assert_eq!(
        unsupported["original"]["uriTemplate"],
        "fixture://owned/{+path}"
    );
    assert!(
        unsupported["unavailableReason"]
            .as_str()
            .unwrap()
            .contains("schema")
    );
    assert!(!peer.dir.path().join("effects").exists());
    session.close().unwrap();
}
