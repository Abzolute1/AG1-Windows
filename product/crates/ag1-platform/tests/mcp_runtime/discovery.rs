use super::*;
use ag1_core::tools::mcp::discovery::NAME;
use ag1_platform::session::{ToolRunner, mcp::discovery::Discovery};

fn scope(f: &Fixture) -> Scope {
    Scope {
        workspace: f.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: 0,
    }
}
fn entry(peer: &super::super::Case, kind: Kind) -> Value {
    json!({"connectionId":peer.connection,"kind":kind,"key":match kind {Kind::Resource=>"fixture://owned/source",Kind::ResourceTemplate=>"fixture://owned/{path}?q={query}",_=>"owned"}})
}
fn selected(peer: &super::super::Case) -> SelectedCapability {
    SelectedCapability {
        connection_id: peer.connection.clone(),
        kind: Kind::Tool,
        key: "owned".into(),
    }
}
async fn attach(f: &mut Fixture, peer: &super::super::Case) -> (Arc<Discovery>, Arc<Accept>) {
    let mut runner = f.files.as_ref().clone();
    let mut registry = runner.registry.as_ref().clone();
    let discovery = Arc::new(
        Discovery::new(
            peer.manager.clone(),
            CatalogScope::GeneralChat,
            scope(f),
            None,
        )
        .unwrap(),
    );
    discovery.register(&mut registry).unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|t| t["name"] == NAME)
        .collect();
    f.host
        .select_tools("discover-tools", "discover-tools", NOW, tools, &ignore)
        .await
        .unwrap();
    runner.registry = Arc::new(registry);
    runner.adapters.mcp_discovery = Some(discovery.clone());
    let approvals = Arc::new(Accept(AtomicUsize::new(0)));
    runner.approvals = approvals.clone();
    f.files = Arc::new(runner);
    (discovery, approvals)
}
fn names(request: &Value) -> Vec<&str> {
    request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_search_inspect_select_execute_and_clear_use_small_next_request_views() {
    for transport in ["stdio", "http"] {
        for kind in [
            Kind::Tool,
            Kind::Resource,
            Kind::ResourceTemplate,
            Kind::Prompt,
        ] {
            let mut peer = super::super::Case::new(
                transport,
                Revision::Current20260728,
                if kind == Kind::Tool {
                    "thousand"
                } else {
                    "content_ok"
                },
            );
            if kind != Kind::Tool {
                peer.content_selection(kind);
            }
            let wire = peer.selection.wire_names().remove(0);
            let args = match kind {
                Kind::Tool => json!({"value":7}),
                Kind::Prompt => json!({"topic":"literal"}),
                Kind::ResourceTemplate => json!({"path":"literal","query":"value"}),
                _ => json!({}),
            };
            let (origin, server) = server(vec![
                calls(vec![(
                    "search",
                    NAME,
                    json!({"action":"search","kind":kind,"query":"owned","limit":1}),
                )]),
                calls(vec![(
                    "inspect",
                    NAME,
                    json!({"action":"inspect","entry":entry(&peer,kind)}),
                )]),
                calls(vec![(
                    "select",
                    NAME,
                    json!({"action":"select","entries":[entry(&peer,kind)]}),
                )]),
                calls(vec![("execute", &wire, args)]),
                calls(vec![("clear", NAME, json!({"action":"clear"}))]),
                stop(),
            ]);
            let mut f = fixture_configured(&origin, "interactive", vec![], None, None, 1);
            let (_, approvals) = attach(&mut f, &peer).await;
            let result = f
                .host
                .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
                .await
                .unwrap();
            assert_eq!(result.generations, 6);
            assert_eq!(
                approvals.0.load(Ordering::SeqCst),
                1,
                "cached metadata is not an external call approval"
            );
            let requests = server.join().unwrap();
            for i in [0, 1, 2, 5] {
                assert_eq!(names(&requests[i]), vec![NAME]);
            }
            for i in [3, 4] {
                assert_eq!(names(&requests[i]), vec![NAME, &wire]);
            }
            assert!(requests.iter().all(|r| r["tools"].to_string().len() < 8192));
            assert!(!requests[0].to_string().contains("owned_unselected_"));
            let results = messages(&f);
            assert_eq!(results.len(), 5);
            assert!(
                results.iter().all(|v| v["message"]["isError"] == false),
                "{results:?}"
            );
            assert_eq!(
                results[0]["message"]["details"]["mcpDiscovery"]["matches"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert!(results[1]["message"]["details"]["mcpDiscovery"]["definitions"][0]["metadata"]["parameters"].is_object());
            assert_eq!(
                results[3]["message"]["details"]["mcp"]["identity"]["binding"]["serverId"],
                peer.server
            );
            // Eviction drops schemas, not source-correlated historical tool results.
            assert!(requests[5]["messages"].to_string().contains(&wire));
            let events = std::fs::read_to_string(peer.dir.path().join("events.jsonl")).unwrap();
            assert_eq!(events.lines().filter(|l|serde_json::from_str::<Value>(l).unwrap()["method"]==kind.method()).count(),1);
            assert!(f.host.settled().unwrap());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selecting_does_not_authorize_an_unadvertised_call_in_the_same_batch() {
    let peer = super::super::Case::new("stdio", Revision::Current20260728, "structured");
    let wire = peer.selection.wire_names().remove(0);
    let (origin, server) = server(vec![
        calls(vec![
            (
                "select",
                NAME,
                json!({"action":"select","entries":[entry(&peer,Kind::Tool)]}),
            ),
            ("smuggled", &wire, json!({"value":7})),
        ]),
        stop(),
    ]);
    let mut f = fixture(&origin, "interactive");
    let (_, approvals) = attach(&mut f, &peer).await;
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(approvals.0.load(Ordering::SeqCst), 0);
    assert!(!peer.dir.path().join("effects").exists());
    assert_eq!(messages(&f)[1]["message"]["isError"], true);
    let requests = server.join().unwrap();
    assert_eq!(names(&requests[0]), vec![NAME]);
    assert_eq!(names(&requests[1]), vec![NAME, &wire]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacing_selection_while_resolving_discards_the_unsent_request() {
    let peer = super::super::Case::new("http", Revision::Current20260728, "structured");
    let (origin, server) = server(vec![stop()]);
    let mut f = fixture(&origin, "interactive");
    let (discovery, _) = attach(&mut f, &peer).await;
    discovery.select(&scope(&f), &[selected(&peer)]).unwrap();
    let resolver = Arc::new(Resolver::new(true));
    let host = f.host.clone();
    let runner = f.files.clone();
    let resolving = resolver.clone();
    let run = tokio::spawn(async move { host.run(input(), &*resolving, runner, &ignore).await });
    resolver.entered.notified().await;
    discovery.select(&scope(&f), &[]).unwrap();
    resolver.release.notify_one();
    run.await.unwrap().unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(names(&requests[0]), vec![NAME]);
    assert!(!peer.dir.path().join("effects").exists());
}

struct ClearOnApproval {
    discovery: Arc<Discovery>,
    scope: Scope,
    count: AtomicUsize,
}
impl ApprovalHandler for ClearOnApproval {
    fn approve(&self, request: &ApprovalRequest, _: &AtomicBool) -> Result<bool, DomainError> {
        assert_eq!(request.display["source"], "mcp");
        self.count.fetch_add(1, Ordering::SeqCst);
        self.discovery.select(&self.scope, &[])?;
        Ok(true)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selection_changed_during_exact_approval_cannot_send_old_call() {
    let peer = super::super::Case::new("stdio", Revision::Current20260728, "structured");
    let wire = peer.selection.wire_names().remove(0);
    let (origin, server) = server(vec![
        calls(vec![("old", &wire, json!({"value":7}))]),
        stop(),
    ]);
    let mut f = fixture(&origin, "interactive");
    let (discovery, _) = attach(&mut f, &peer).await;
    discovery.select(&scope(&f), &[selected(&peer)]).unwrap();
    let mut runner = f.files.as_ref().clone();
    let approval = Arc::new(ClearOnApproval {
        discovery,
        scope: scope(&f),
        count: AtomicUsize::new(0),
    });
    runner.approvals = approval.clone();
    f.host
        .run(input(), &Resolver::new(false), Arc::new(runner), &ignore)
        .await
        .unwrap();
    assert_eq!(approval.count.load(Ordering::SeqCst), 1);
    assert!(!peer.dir.path().join("effects").exists());
    assert_eq!(messages(&f)[0]["message"]["isError"], true);
    assert_eq!(names(&server.join().unwrap()[1]), vec![NAME]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_scope_and_rejected_selection_preserve_the_original_snapshot() {
    let peer = super::super::Case::new("stdio", Revision::Current20260728, "structured");
    let mut f = fixture("http://127.0.0.1:1", "interactive");
    let (discovery, _) = attach(&mut f, &peer).await;
    let owner = scope(&f);
    discovery.select(&owner, &[selected(&peer)]).unwrap();
    let captured = f.files.mcp_request_runner(&owner).unwrap().unwrap();
    let view = captured
        .prepare_mcp_request(&owner, &f.host.configuration().unwrap().tools)
        .unwrap();
    for other in [
        Scope {
            workspace: "foreign".into(),
            ..owner.clone()
        },
        Scope {
            session: "foreign".into(),
            ..owner.clone()
        },
        Scope {
            epoch: 2,
            ..owner.clone()
        },
    ] {
        assert!(discovery.select(&other, &[]).is_err());
        assert!(f.files.mcp_request_runner(&other).is_err());
    }
    let bad = SelectedCapability {
        key: "absent".into(),
        ..selected(&peer)
    };
    assert!(discovery.select(&owner, &[bad]).is_err());
    assert!(
        discovery
            .select(&owner, &[selected(&peer), selected(&peer)])
            .is_err()
    );
    captured.validate_mcp_request(&owner, &view).unwrap();
    // Native mode controllers can restrict this view without a rebuild loop.
    captured.validate_mcp_request(&owner, &[]).unwrap();
    let mut forged = view.clone();
    forged.last_mut().unwrap()["description"] = json!("forged");
    assert!(captured.validate_mcp_request(&owner, &forged).is_err());
    discovery.select(&owner, &[]).unwrap();
    assert!(captured.validate_mcp_request(&owner, &view).is_err());
}
