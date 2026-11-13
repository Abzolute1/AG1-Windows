#![allow(dead_code, unused_imports)] // Shared native fixture also supports broader session tests.
include!("../support/session_fixture.rs");
mod composition;
mod discovery;
mod management;
use ag1_core::tools::executor::ExecutionArtifacts;
use ag1_core::tools::{approval::ApprovalRequest, mcp::Kind, registry::Scope};
use ag1_platform::{
    mcp::{Revision, catalog::CatalogScope, manager::SelectedCapability},
    session::{files::ApprovalHandler, mcp::SessionCapabilities},
};
struct Accept(AtomicUsize);
impl ApprovalHandler for Accept {
    fn approve(&self, request: &ApprovalRequest, _: &AtomicBool) -> Result<bool, DomainError> {
        assert_eq!(request.display["source"], "mcp");
        assert_eq!(request.display["risk"], "unknown");
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}
async fn attach_mcp(f: &mut Fixture, peer: &super::Case, kind: Kind, epoch: u64) -> Arc<Accept> {
    let mut runner = f.files.as_ref().clone();
    let mut registry = runner.registry.as_ref().clone();
    let key = match kind {
        Kind::Tool | Kind::Prompt => "owned",
        Kind::Resource => "fixture://owned/source",
        Kind::ResourceTemplate => "fixture://owned/{path}?q={query}",
    };
    let mcp = Arc::new(
        SessionCapabilities::new(
            peer.manager.clone(),
            CatalogScope::GeneralChat,
            Scope {
                workspace: f.workspace.id().as_str().into(),
                session: "session".into(),
                epoch,
                revision: 0,
            },
            &mut registry,
            &[SelectedCapability {
                connection_id: peer.connection.clone(),
                kind,
                key: key.into(),
            }],
            None,
        )
        .unwrap(),
    );
    let names = mcp.wire_names();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|t| names.iter().any(|n| t["name"] == *n))
        .collect();
    f.host
        .select_tools("mcp-tools", "mcp-tools", NOW, tools, &ignore)
        .await
        .unwrap();
    runner.adapters.mcp = Some(mcp);
    runner.registry = Arc::new(registry);
    let approval = Arc::new(Accept(AtomicUsize::new(0)));
    runner.approvals = approval.clone();
    f.files = Arc::new(runner);
    approval
}
fn messages(f: &Fixture) -> Vec<Value> {
    ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap()
    .into_iter()
    .filter(|v| v["message"]["role"] == "toolResult")
    .collect()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_native_loop_uses_mcp_approval_executor_context_and_provenance() {
    for transport in ["stdio", "http"] {
        for (kind, mode, args) in [
            (Kind::Tool, "structured", json!({"value":7})),
            (Kind::Tool, "thousand", json!({"value":7})),
            (Kind::Resource, "content_ok", json!({})),
            (Kind::Prompt, "content_ok", json!({"topic":"literal"})),
        ] {
            let mut peer = super::Case::new(transport, Revision::Current20260728, mode);
            if kind != Kind::Tool {
                peer.content_selection(kind);
            }
            let name = peer.selection.wire_names().remove(0);
            let (origin, server) = server(vec![calls(vec![("mcp-call", &name, args)]), stop()]);
            let mut f = fixture_configured(&origin, "disabled", vec![], None, None, 1);
            let approval = attach_mcp(&mut f, &peer, kind, 1).await;
            let result = f
                .host
                .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
                .await
                .unwrap();
            assert_eq!(result.generations, 2);
            assert_eq!(
                approval.0.load(Ordering::SeqCst),
                1,
                "generic bypass must not bypass MCP approval"
            );
            let requests = server.join().unwrap();
            assert_eq!(requests[0]["tools"].as_array().unwrap().len(), 1);
            assert!(!requests[0].to_string().contains("owned_unselected_"));
            assert!(requests[0]["tools"].to_string().len() < 4096);
            assert_eq!(requests[0]["tools"][0]["function"]["name"], name);
            let result = messages(&f).remove(0);
            assert_eq!(
                result["message"]["isError"], false,
                "{transport}/{kind:?}: {result}"
            );
            assert_eq!(
                requests[1]["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|v| v["role"] == "tool")
                    .count(),
                1
            );
            assert_eq!(
                result["message"]["details"]["mcp"]["identity"]["binding"]["serverId"],
                peer.server
            );
            assert_eq!(
                result["message"]["details"]["mcp"]["identity"]["originalName"],
                if kind == Kind::Resource {
                    "fixture://owned/source"
                } else {
                    "owned"
                }
            );
            let original: ag1_core::context::ContextArtifact = serde_json::from_value(
                result["message"]["details"]["mcp"]["originalResponse"].clone(),
            )
            .unwrap();
            let artifacts = ag1_storage::context::ContextStore::new(
                &f.store,
                SessionId::new("session").unwrap(),
                "session",
                "reader",
                NOW,
                Default::default(),
            )
            .unwrap();
            let retained = artifacts.read_json(&original).unwrap();
            assert!(retained["result"].is_object());
            let journal = f
                .store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap();
            assert!(
                journal
                    .entries
                    .iter()
                    .any(|e| String::from_utf8_lossy(&e.normalized).contains("mcp"))
            );
            assert!(f.host.settled().unwrap());
            let events = std::fs::read_to_string(peer.dir.path().join("events.jsonl")).unwrap();
            assert_eq!(events.lines().filter(|l|serde_json::from_str::<Value>(l).unwrap()["method"]==kind.method()).count(),1);
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_epoch_and_planning_mode_never_send_mcp_calls() {
    for (epoch, mode) in [(2, "disabled"), (1, "planning")] {
        let peer = super::Case::new("stdio", Revision::Current20260728, "content_ok");
        let name = peer.selection.wire_names().remove(0);
        let (origin, server) = server(vec![calls(vec![("mcp-call", &name, json!({}))]), stop()]);
        let mut f = fixture(&origin, mode);
        let approval = attach_mcp(&mut f, &peer, Kind::Resource, epoch).await;
        f.host
            .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
            .await
            .unwrap();
        assert_eq!(approval.0.load(Ordering::SeqCst), 0);
        assert_eq!(messages(&f)[0]["message"]["isError"], true);
        assert!(!peer.dir.path().join("effects").exists());
        assert_eq!(server.join().unwrap().len(), 2);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_cancel_during_refresh_prevents_send_and_drains_single_worker() {
    for transport in ["stdio", "http"] {
        let peer = super::Case::new(transport, Revision::Current20260728, "content_slow_refresh");
        let name = peer.selection.wire_names().remove(0);
        let (origin, server) = server(vec![calls(vec![("mcp-call", &name, json!({}))])]);
        let mut f = fixture_configured(&origin, "disabled", vec![], None, None, 1);
        attach_mcp(&mut f, &peer, Kind::Resource, 1).await;
        let host = f.host.clone();
        let runner = f.files.clone();
        let run = tokio::spawn(async move {
            host.run(input(), &Resolver::new(false), runner, &ignore)
                .await
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !peer.dir.path().join("refreshing").exists() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        f.host.cancel().unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(3), run)
            .await
            .unwrap()
            .unwrap();
        assert!(!peer.dir.path().join("effects").exists());
        assert!(f.host.settled().unwrap());
        assert_eq!(server.join().unwrap().len(), 1);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_cancel_after_send_retains_uncertainty_without_private_result_delivery() {
    let peer = super::Case::new("http", Revision::Current20260728, "content_hang");
    let name = peer.selection.wire_names().remove(0);
    let (origin, server) = server(vec![calls(vec![("mcp-call", &name, json!({}))])]);
    let mut f = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    attach_mcp(&mut f, &peer, Kind::Resource, 1).await;
    let host = f.host.clone();
    let runner = f.files.clone();
    let run = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), runner, &ignore)
            .await
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !peer.dir.path().join("effects").exists() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    f.host.cancel().unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap();
    assert!(f.host.settled().unwrap());
    let results = messages(&f);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["message"]["isError"], true);
    assert!(!results[0].to_string().contains("Owned resource bytes"));
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disable_during_provider_preparation_rebuilds_unsent_request_without_mcp_schemas() {
    let peer = super::Case::new("stdio", Revision::Current20260728, "content_ok");
    let (origin, server) = server(vec![stop()]);
    let mut f = fixture(&origin, "disabled");
    attach_mcp(&mut f, &peer, Kind::Resource, 1).await;
    let resolver = Arc::new(Resolver::new(true));
    let host = f.host.clone();
    let runner = f.files.clone();
    let resolving = resolver.clone();
    let run = tokio::spawn(async move { host.run(input(), &*resolving, runner, &ignore).await });
    resolver.entered.notified().await;
    peer.manager
        .set_availability(
            &peer.server,
            2,
            ag1_platform::mcp::definitions::Availability::default(),
        )
        .unwrap();
    resolver.release.notify_one();
    run.await.unwrap().unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .get("tools")
            .is_none_or(|t| t.as_array().is_some_and(Vec::is_empty))
    );
    assert!(!peer.dir.path().join("effects").exists());
    assert!(f.host.settled().unwrap());
}

#[test]
fn session_capability_cannot_be_used_by_another_workspace_session_or_epoch() {
    let peer = super::Case::new("stdio", Revision::Current20260728, "content_ok");
    let mut registry = Registry::builtins().unwrap();
    let bound = Scope {
        workspace: "owner".into(),
        session: "session".into(),
        epoch: 1,
        revision: 0,
    };
    let port = SessionCapabilities::new(
        peer.manager.clone(),
        CatalogScope::GeneralChat,
        bound.clone(),
        &mut registry,
        &[SelectedCapability {
            connection_id: peer.connection.clone(),
            kind: Kind::Resource,
            key: "fixture://owned/source".into(),
        }],
        None,
    )
    .unwrap();
    for scope in [
        Scope {
            workspace: "other".into(),
            ..bound.clone()
        },
        Scope {
            session: "other".into(),
            ..bound.clone()
        },
        Scope {
            epoch: 2,
            ..bound.clone()
        },
    ] {
        assert_eq!(
            port.validate(&scope, &registry),
            Err(DomainError::PermissionDenied)
        );
    }
    assert!(
        port.validate(
            &Scope {
                revision: 9,
                ..bound.clone()
            },
            &registry
        )
        .is_ok()
    );
    assert!(
        SessionCapabilities::new(
            peer.manager.clone(),
            CatalogScope::Workspace("foreign".into()),
            bound,
            &mut registry,
            &[],
            None
        )
        .is_err()
    );
}
