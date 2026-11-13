#![cfg(target_os = "linux")]
use ag1_core::DomainError;
#[path = "mcp_manager/access_import.rs"]
mod access_import;
#[path = "mcp_manager/private_credentials.rs"]
mod private_credentials;
#[path = "mcp_manager/public_registry.rs"]
mod public_registry;
#[path = "mcp_manager/trust.rs"]
mod trust;
use ag1_platform::mcp::{
    Revision,
    catalog::CatalogScope,
    definitions::{Availability, Configuration, Draft, Provenance, import_project},
    discovery::DiscoveryLimits,
    manager::{ConnectionConsent, ConnectionHealth, Manager},
};
use ag1_storage::Store;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};

fn manager(path: &Path) -> Manager {
    Manager::open(
        Store::open(path).unwrap(),
        Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
        DiscoveryLimits::new(Duration::from_secs(2)).unwrap(),
    )
    .unwrap()
}
fn remote(name: &str) -> Draft {
    Draft {
        name: name.into(),
        configuration: Configuration::Http {
            url: "https://example.invalid/mcp".into(),
            loopback_development: false,
        },
        protocol: Revision::Current20260728.as_str().into(),
        auth_profile: None,
        provenance: Provenance::Manual,
    }
}
fn local(path: &Path, mode: &str, revision: Revision) -> Draft {
    let script = path.join("peer.py");
    std::fs::write(&script, include_str!("fixtures/mcp_stdio.py")).unwrap();
    Draft {
        name: "owned local".into(),
        configuration: Configuration::Local {
            executable: "/usr/bin/python3".into(),
            arguments: vec![
                "-I".into(),
                script.to_str().unwrap().into(),
                mode.into(),
                revision.as_str().into(),
            ],
            cwd: path.to_str().unwrap().into(),
            public_environment: BTreeMap::new(),
            secret_environment: BTreeSet::new(),
        },
        protocol: revision.as_str().into(),
        auth_profile: None,
        provenance: Provenance::Manual,
    }
}
fn enable(m: &Manager, id: &str) {
    let rev = m
        .records()
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
        .revision;
    m.set_availability(
        id,
        rev,
        Availability {
            enabled: true,
            general_chat: true,
            all_workspaces: true,
            ..Default::default()
        },
    )
    .unwrap();
}
fn wait(m: &Manager, id: &str, health: ConnectionHealth) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let status = m.status(id).unwrap();
        if status.health == health {
            return;
        }
        assert!(
            Instant::now() < deadline && status.health != ConnectionHealth::Failed,
            "{status:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn events(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
#[test]
fn imports_and_reopen_are_inert_and_grants_are_separate_from_definitions() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let raw = br#"{"mcpServers":{"proposed":{"command":"/missing/server","args":["literal;$(must-not-run)"]}}}"#;
    let drafts = import_project(raw, "/owned/.mcp.json", dir.path().to_str().unwrap()).unwrap();
    assert!(
        matches!(&drafts[0].provenance, Provenance::Project { entry, .. } if entry == "proposed")
    );
    let id;
    {
        let m = manager(&state);
        id = m.add_drafts(drafts).unwrap().remove(0);
        let record = m.records().unwrap().remove(0);
        assert_eq!(record.availability, Availability::default());
        assert!(matches!(
            m.review_connection(&id, CatalogScope::GeneralChat),
            Err(DomainError::PermissionDenied)
        ));
        assert!(events(dir.path()).is_empty());
    }
    let m = manager(&state);
    assert_eq!(m.records().unwrap()[0].id, id);
    assert!(!m.records().unwrap()[0].availability.enabled);
    assert!(events(dir.path()).is_empty());
}
#[test]
fn hostile_imports_fail_atomically_without_silent_mapping_or_secret_copy() {
    for raw in [
        r#"{"mcpServers":{"x":{"command":"/bin/false","env":{"TOKEN":"synthetic-secret"}}}}"#,
        r#"{"mcpServers":{"x":{"command":"npx","args":["-y","unreviewed"]}}}"#,
        r#"{"mcpServers":{"x":{"command":"/bin/false","command":"/bin/true"}}}"#,
        r#"{"mcpServers":{"x":{"command":"/bin/false","cwd":123}}}"#,
        r#"{"mcpServers":{"good":{"command":"/bin/false"},"bad":{"url":"http://127.0.0.1/mcp"}}}"#,
    ] {
        let error = import_project(raw.as_bytes(), "/project/.mcp.json", "/project").unwrap_err();
        assert!(!format!("{error:?}").contains("synthetic-secret"));
    }
    assert!(
        import_project(
            &vec![b' '; 256 * 1024 + 1],
            "/project/.mcp.json",
            "/project"
        )
        .is_err()
    );
}
#[test]
fn workspace_denial_overrides_global_and_general_chat_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let id = m.add_drafts(vec![remote("one")]).unwrap().remove(0);
    m.set_availability(
        &id,
        1,
        Availability {
            enabled: true,
            all_workspaces: true,
            workspaces: BTreeSet::from(["denied".into()]),
            denied_workspaces: BTreeSet::from(["denied".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        m.review_connection(&id, CatalogScope::Workspace("allowed".into()))
            .is_ok()
    );
    assert!(
        m.review_connection(&id, CatalogScope::Workspace("denied".into()))
            .is_err()
    );
    assert!(m.review_connection(&id, CatalogScope::GeneralChat).is_err());
}
#[test]
fn edits_revocation_cross_owner_and_remove_readd_refuse_old_reviews() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("first"));
    let other = manager(&dir.path().join("second"));
    let id = m.add_drafts(vec![remote("same name")]).unwrap().remove(0);
    enable(&m, &id);
    let review = m.review_connection(&id, CatalogScope::GeneralChat).unwrap();
    assert!(
        other
            .connect(review, ConnectionConsent::RemoteEndpoint)
            .is_err()
    );
    let review = m.review_connection(&id, CatalogScope::GeneralChat).unwrap();
    m.edit(&id, 2, remote("changed")).unwrap();
    assert!(
        m.connect(review, ConnectionConsent::RemoteEndpoint)
            .is_err()
    );
    assert!(!m.records().unwrap()[0].availability.enabled);
    assert!(m.set_availability(&id, 2, Availability::default()).is_err());
    enable(&m, &id);
    let review = m.review_connection(&id, CatalogScope::GeneralChat).unwrap();
    m.remove(&id, 4).unwrap();
    let replacement = m.add_drafts(vec![remote("same name")]).unwrap().remove(0);
    assert_ne!(id, replacement);
    assert!(
        m.connect(review, ConnectionConsent::RemoteEndpoint)
            .is_err()
    );
}
#[test]
fn profiles_and_private_environment_are_inactive_until_their_native_flow_exists() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let mut auth = remote("profile");
    auth.auth_profile = Some("opaque-mcp-profile".into());
    let mut secret = local(dir.path(), "normal", Revision::Current20260728);
    if let Configuration::Local {
        secret_environment, ..
    } = &mut secret.configuration
    {
        secret_environment.insert("TOKEN".into());
    }
    for draft in [auth, secret] {
        let id = m.add_drafts(vec![draft]).unwrap().remove(0);
        enable(&m, &id);
        assert!(matches!(
            m.review_connection(&id, CatalogScope::GeneralChat),
            Err(DomainError::Unavailable(_))
        ));
    }
    assert!(events(dir.path()).is_empty());
}
#[test]
fn owned_local_manager_discovers_both_revisions_and_disconnect_invalidates_catalog() {
    for revision in [Revision::Legacy20251125, Revision::Current20260728] {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let m = manager(&state);
        let server = m
            .add_drafts(vec![local(dir.path(), "normal", revision)])
            .unwrap()
            .remove(0);
        enable(&m, &server);
        let review = m
            .review_connection(&server, CatalogScope::GeneralChat)
            .unwrap();
        assert!(review.executable_sha256().is_some());
        assert!(events(dir.path()).is_empty());
        let connection = m
            .connect(review, ConnectionConsent::LocalHostAccountExecution)
            .unwrap();
        wait(&m, &connection, ConnectionHealth::Ready);
        let catalog = m.catalog(&connection, &CatalogScope::GeneralChat).unwrap();
        assert_eq!(catalog.tools.len(), 2);
        assert_eq!(catalog.resources.len(), 1);
        assert_eq!(catalog.templates.len(), 1);
        assert_eq!(catalog.prompts.len(), 1);
        assert_eq!(catalog.binding.protocol, revision);
        assert!(
            m.catalog(&connection, &CatalogScope::Workspace("other".into()))
                .is_err()
        );
        let old_generation = catalog.binding.connection_generation;
        m.disconnect(&connection).unwrap();
        assert!(m.catalog(&connection, &CatalogScope::GeneralChat).is_err());
        m.wait_disconnected(&connection).unwrap();
        assert!(m.records().unwrap()[0].availability.enabled);
        let review = m
            .review_connection(&server, CatalogScope::GeneralChat)
            .unwrap();
        let connection = m
            .connect(review, ConnectionConsent::LocalHostAccountExecution)
            .unwrap();
        wait(&m, &connection, ConnectionHealth::Ready);
        assert_ne!(
            old_generation,
            m.catalog(&connection, &CatalogScope::GeneralChat)
                .unwrap()
                .binding
                .connection_generation
        );
        drop(m);
        let count = events(dir.path()).len();
        let m = manager(&state);
        assert!(m.records().unwrap()[0].availability.enabled);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            count,
            events(dir.path()).len(),
            "restart must not reconnect"
        );
        assert!(
            events(dir.path())
                .iter()
                .all(|s| !["tools/call", "resources/read", "prompts/get"].contains(&s.as_str()))
        );
    }
}
#[test]
fn disable_during_start_cancels_promptly_and_never_publishes_late_results() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let id = m
        .add_drafts(vec![local(dir.path(), "hang", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    enable(&m, &id);
    let review = m.review_connection(&id, CatalogScope::GeneralChat).unwrap();
    let connection = m
        .connect(review, ConnectionConsent::LocalHostAccountExecution)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while events(dir.path()).is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let started = Instant::now();
    m.set_availability(&id, 2, Availability::default()).unwrap();
    assert!(m.catalog(&connection, &CatalogScope::GeneralChat).is_err());
    m.wait_disconnected(&connection).unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(m.review_connection(&id, CatalogScope::GeneralChat).is_err());
}
#[test]
fn revoking_one_server_leaves_an_unrelated_connection_available() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let m = manager(&dir.path().join("state"));
    let ids = m
        .add_drafts(vec![
            local(&a, "normal", Revision::Current20260728),
            local(&b, "normal", Revision::Current20260728),
        ])
        .unwrap();
    let mut connections = vec![];
    for id in &ids {
        enable(&m, id);
        connections.push(
            m.connect(
                m.review_connection(id, CatalogScope::GeneralChat).unwrap(),
                ConnectionConsent::LocalHostAccountExecution,
            )
            .unwrap(),
        );
    }
    for id in &connections {
        wait(&m, id, ConnectionHealth::Ready);
    }
    m.remove(&ids[0], 2).unwrap();
    m.wait_disconnected(&connections[0]).unwrap();
    assert!(
        m.catalog(&connections[1], &CatalogScope::GeneralChat)
            .is_ok()
    );
}
#[test]
fn local_target_replacement_and_wrong_consent_cannot_launch() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let exe = dir.path().join("python");
    std::os::unix::fs::symlink("/usr/bin/python3", &exe).unwrap();
    let mut draft = local(dir.path(), "normal", Revision::Current20260728);
    if let Configuration::Local { executable, .. } = &mut draft.configuration {
        *executable = exe.to_str().unwrap().into();
    }
    let id = m.add_drafts(vec![draft]).unwrap().remove(0);
    enable(&m, &id);
    let wrong = m.review_connection(&id, CatalogScope::GeneralChat).unwrap();
    assert!(m.connect(wrong, ConnectionConsent::RemoteEndpoint).is_err());
    let review = m.review_connection(&id, CatalogScope::GeneralChat).unwrap();
    std::fs::remove_file(&exe).unwrap();
    std::os::unix::fs::symlink("/usr/bin/false", &exe).unwrap();
    assert!(
        m.connect(review, ConnectionConsent::LocalHostAccountExecution)
            .is_err()
    );
    assert!(events(dir.path()).is_empty());
}
#[test]
fn review_and_definition_budgets_are_bounded_and_released() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let id = m.add_drafts(vec![remote("one")]).unwrap().remove(0);
    enable(&m, &id);
    let mut reviews = vec![];
    for _ in 0..32 {
        reviews.push(m.review_connection(&id, CatalogScope::GeneralChat).unwrap());
    }
    assert!(matches!(
        m.review_connection(&id, CatalogScope::GeneralChat),
        Err(DomainError::LimitExceeded(_))
    ));
    reviews.pop();
    assert!(m.review_connection(&id, CatalogScope::GeneralChat).is_ok());
    for _ in 0..3 {
        m.add_drafts((0..32).map(|_| remote("same name")).collect())
            .unwrap();
    }
    m.add_drafts((0..31).map(|_| remote("same name")).collect())
        .unwrap();
    assert_eq!(m.records().unwrap().len(), 128);
    assert!(m.add_drafts(vec![remote("overflow")]).is_err());
}
#[test]
fn manager_drop_reaps_escaped_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let pid;
    {
        let m = manager(&dir.path().join("state"));
        let id = m
            .add_drafts(vec![local(
                dir.path(),
                "descendant",
                Revision::Current20260728,
            )])
            .unwrap()
            .remove(0);
        enable(&m, &id);
        let connection = m
            .connect(
                m.review_connection(&id, CatalogScope::GeneralChat).unwrap(),
                ConnectionConsent::LocalHostAccountExecution,
            )
            .unwrap();
        wait(&m, &connection, ConnectionHealth::Ready);
        pid = std::fs::read_to_string(dir.path().join("descendant.pid")).unwrap();
    }
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
}
#[test]
fn active_connection_budget_releases_only_after_owned_shutdown_and_scope_cancel_is_independent() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let server = m
        .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    enable(&m, &server);
    let mut connections = vec![];
    for index in 0..4 {
        let scope = CatalogScope::Workspace(format!("workspace-{index}"));
        let review = m.review_connection(&server, scope).unwrap();
        connections.push(
            m.connect(review, ConnectionConsent::LocalHostAccountExecution)
                .unwrap(),
        );
    }
    for id in &connections {
        wait(&m, id, ConnectionHealth::Ready);
    }
    let review = m
        .review_connection(&server, CatalogScope::GeneralChat)
        .unwrap();
    assert!(matches!(
        m.connect(review, ConnectionConsent::LocalHostAccountExecution),
        Err(DomainError::LimitExceeded(_))
    ));
    let duplicate = m
        .review_connection(&server, CatalogScope::Workspace("workspace-0".into()))
        .unwrap();
    assert!(
        m.connect(duplicate, ConnectionConsent::LocalHostAccountExecution)
            .is_err()
    );
    m.disconnect(&connections[0]).unwrap();
    m.wait_disconnected(&connections[0]).unwrap();
    assert_eq!(
        m.status(&connections[0]).unwrap().health,
        ConnectionHealth::Stopped
    );
    assert!(
        m.catalog(
            &connections[1],
            &CatalogScope::Workspace("workspace-1".into())
        )
        .is_ok()
    );
    let fresh = m
        .connect(
            m.review_connection(&server, CatalogScope::GeneralChat)
                .unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &fresh, ConnectionHealth::Ready);
}
#[test]
fn failed_discovery_has_no_partial_catalog_and_diagnostics_survive_a_later_connection() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let server = m
        .add_drafts(vec![local(
            dir.path(),
            "catalog_duplicate",
            Revision::Current20260728,
        )])
        .unwrap()
        .remove(0);
    enable(&m, &server);
    let connection = m
        .connect(
            m.review_connection(&server, CatalogScope::GeneralChat)
                .unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &connection, ConnectionHealth::Failed);
    assert!(m.catalog(&connection, &CatalogScope::GeneralChat).is_err());
    assert!(m.status(&connection).unwrap().diagnostic.is_some());
    m.edit(
        &server,
        2,
        local(dir.path(), "normal", Revision::Current20260728),
    )
    .unwrap();
    enable(&m, &server);
    let fresh = m
        .connect(
            m.review_connection(&server, CatalogScope::GeneralChat)
                .unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &fresh, ConnectionHealth::Ready);
    assert!(
        m.recent_connections()
            .unwrap()
            .iter()
            .any(|s| s.id == connection && s.health == ConnectionHealth::Failed)
    );
}
#[test]
fn explicit_test_can_inspect_an_inactive_draft_without_enabling_model_catalogs() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let server = m
        .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    let review = m
        .review_test_connection(&server, CatalogScope::GeneralChat)
        .unwrap();
    let connection = m
        .connect(review, ConnectionConsent::LocalHostAccountExecution)
        .unwrap();
    wait(&m, &connection, ConnectionHealth::Ready);
    assert_eq!(
        m.inspect_catalog(&connection, &CatalogScope::GeneralChat)
            .unwrap()
            .tools
            .len(),
        2
    );
    assert!(m.catalog(&connection, &CatalogScope::GeneralChat).is_err());
    assert!(!m.records().unwrap()[0].availability.enabled);
    enable(&m, &server);
    assert!(
        m.inspect_catalog(&connection, &CatalogScope::GeneralChat)
            .is_err(),
        "availability edit retires test lease too"
    );
    m.wait_disconnected(&connection).unwrap();
}
#[test]
fn aggregate_catalog_budget_and_cross_kind_changes_prevent_partial_publication() {
    for mode in ["manager_budget", "manager_late_change"] {
        let dir = tempfile::tempdir().unwrap();
        let m = manager(&dir.path().join("state"));
        let id = m
            .add_drafts(vec![local(dir.path(), mode, Revision::Current20260728)])
            .unwrap()
            .remove(0);
        enable(&m, &id);
        let connection = m
            .connect(
                m.review_connection(&id, CatalogScope::GeneralChat).unwrap(),
                ConnectionConsent::LocalHostAccountExecution,
            )
            .unwrap();
        wait(&m, &connection, ConnectionHealth::Failed);
        assert!(
            m.inspect_catalog(&connection, &CatalogScope::GeneralChat)
                .is_err()
        );
        m.wait_disconnected(&connection).unwrap();
    }
}
#[test]
fn nonregular_launch_target_is_refused_without_waiting_for_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let fifo = dir.path().join("fifo");
    let encoded = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(encoded.as_ptr(), 0o600) }, 0);
    let mut draft = local(dir.path(), "normal", Revision::Current20260728);
    if let Configuration::Local { executable, .. } = &mut draft.configuration {
        *executable = fifo.to_str().unwrap().into();
    }
    let id = m.add_drafts(vec![draft]).unwrap().remove(0);
    enable(&m, &id);
    assert!(m.review_connection(&id, CatalogScope::GeneralChat).is_err());
}
#[test]
fn uncertain_storage_ack_retires_the_owner_and_cancels_existing_connections() {
    use ag1_core::Clock;
    use ag1_storage::{Boundary, FaultInjector, StorageError, StorageResult};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct TestClock;
    impl Clock for TestClock {
        fn now_ms(&self) -> u64 {
            1
        }
    }
    struct Fault(AtomicBool);
    impl FaultInjector for Fault {
        fn check(&self, boundary: Boundary) -> StorageResult<()> {
            if boundary == Boundary::AfterCommit && self.0.swap(false, Ordering::AcqRel) {
                return Err(StorageError::Injected(boundary));
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let fault = Arc::new(Fault(AtomicBool::new(false)));
    let store = Store::open_with(&state, Arc::new(TestClock), fault.clone()).unwrap();
    let m = Manager::open(
        store,
        Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
        DiscoveryLimits::default(),
    )
    .unwrap();
    let id = m
        .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    enable(&m, &id);
    let pending = m.review_connection(&id, CatalogScope::GeneralChat).unwrap();
    let connection = m
        .connect(
            m.review_connection(&id, CatalogScope::GeneralChat).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &connection, ConnectionHealth::Ready);
    fault.0.store(true, Ordering::Release);
    assert!(m.set_availability(&id, 2, Availability::default()).is_err());
    assert!(m.catalog(&connection, &CatalogScope::GeneralChat).is_err());
    assert!(
        m.connect(pending, ConnectionConsent::LocalHostAccountExecution)
            .is_err()
    );
    m.wait_disconnected(&connection).unwrap();
    drop(m);
    let reopened = manager(&state);
    assert!(!reopened.records().unwrap()[0].availability.enabled);
}

#[test]
fn thousand_tools_are_searched_without_schema_injection_and_only_selected_schemas_enter_registry() {
    use ag1_core::tools::registry::{Origin, Registry};
    use ag1_platform::mcp::manager::SelectedTool;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let server = m
        .add_drafts(vec![local(
            dir.path(),
            "selection_scale",
            Revision::Current20260728,
        )])
        .unwrap()
        .remove(0);
    enable(&m, &server);
    let connection = m
        .connect(
            m.review_connection(&server, CatalogScope::GeneralChat)
                .unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &connection, ConnectionHealth::Ready);
    let methods = events(dir.path());
    let page = m.search_tools(&CatalogScope::GeneralChat, "", 5).unwrap();
    assert_eq!(page.searched_tools, 1000);
    assert_eq!(page.matches.len(), 5);
    assert!(page.truncated);
    assert!(page.encoded_bytes <= 32 * 1024);
    assert_eq!(
        m.search_tools(&CatalogScope::Workspace("other".into()), "", 5)
            .unwrap()
            .searched_tools,
        0
    );
    let mut registry = Registry::builtins().unwrap();
    let baseline = registry.definitions().count();
    let requests = vec![
        SelectedTool {
            connection_id: connection.clone(),
            original_name: "tool-0000".into(),
        },
        SelectedTool {
            connection_id: connection.clone(),
            original_name: "tool-0998".into(),
        },
    ];
    let selection = m
        .select_tools(&mut registry, CatalogScope::GeneralChat, &requests)
        .unwrap();
    assert_eq!(registry.definitions().count(), baseline + 2);
    assert_eq!(
        registry
            .definitions()
            .filter(|d| d.origin == Origin::Mcp)
            .count(),
        2
    );
    assert!(selection.schema_bytes() < 4096);
    assert_eq!(selection.wire_names().len(), 2);
    let name = selection.wire_names().remove(0);
    assert!(
        selection
            .prepare(&m, &registry, "op", "call", &name, br#"{"count":"1"}"#)
            .is_err()
    );
    let call = selection
        .prepare(&m, &registry, "op", "call", &name, br#"{"count":1}"#)
        .unwrap();
    assert!(call.mcp_identity().is_some());
    let revision = registry.revision();
    let again = m
        .select_tools(&mut registry, CatalogScope::GeneralChat, &requests)
        .unwrap();
    assert_eq!(registry.revision(), revision);
    selection.validate(&m, &registry).unwrap();
    again.validate(&m, &registry).unwrap();
    let unsupported = m
        .search_tools(&CatalogScope::GeneralChat, "tool-0999", 1)
        .unwrap();
    assert_eq!(
        unsupported.matches[0].schema_support,
        ag1_platform::mcp::catalog::SchemaSupport::UnsupportedInput
    );
    assert!(
        m.select_tools(
            &mut registry,
            CatalogScope::GeneralChat,
            &[SelectedTool {
                connection_id: connection.clone(),
                original_name: "tool-0999".into()
            }]
        )
        .is_err()
    );
    assert_eq!(registry.revision(), revision);
    assert_eq!(
        events(dir.path()),
        methods,
        "cached search/selection must send no protocol requests"
    );
    m.disconnect(&connection).unwrap();
    assert!(selection.validate(&m, &registry).is_err());
    assert!(selection.validate_call(&m, &registry, &call).is_err());
    assert!(
        m.search_tools(&CatalogScope::GeneralChat, "", 5)
            .unwrap()
            .matches
            .is_empty()
    );
    m.wait_disconnected(&connection).unwrap();
}
#[test]
fn inactive_inspection_catalogs_and_foreign_owners_cannot_supply_selected_tools() {
    use ag1_core::tools::registry::Registry;
    use ag1_platform::mcp::manager::SelectedTool;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir.path().join("state"));
    let other = manager(&dir.path().join("other"));
    let server = m
        .add_drafts(vec![local(dir.path(), "normal", Revision::Current20260728)])
        .unwrap()
        .remove(0);
    let connection = m
        .connect(
            m.review_test_connection(&server, CatalogScope::GeneralChat)
                .unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &connection, ConnectionHealth::Ready);
    let mut registry = Registry::builtins().unwrap();
    let request = [SelectedTool {
        connection_id: connection.clone(),
        original_name: "first".into(),
    }];
    assert!(
        m.select_tools(&mut registry, CatalogScope::GeneralChat, &request)
            .is_err()
    );
    assert!(
        m.search_tools(&CatalogScope::GeneralChat, "", 5)
            .unwrap()
            .matches
            .is_empty()
    );
    enable(&m, &server);
    m.wait_disconnected(&connection).unwrap();
    let connection = m
        .connect(
            m.review_connection(&server, CatalogScope::GeneralChat)
                .unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &connection, ConnectionHealth::Ready);
    let selection = m
        .select_tools(
            &mut registry,
            CatalogScope::GeneralChat,
            &[SelectedTool {
                connection_id: connection.clone(),
                original_name: "first".into(),
            }],
        )
        .unwrap();
    assert!(selection.validate(&other, &registry).is_err());
    m.edit(
        &server,
        2,
        local(dir.path(), "normal", Revision::Current20260728),
    )
    .unwrap();
    assert!(selection.validate(&m, &registry).is_err());
    m.wait_disconnected(&connection).unwrap();
}
