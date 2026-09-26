use super::*;
use ag1_core::{SecretBytes, tools::registry::Scope};
use ag1_platform::{
    mcp::{
        auth::{Material, Status, Vault},
        manager::{PrivateEntry, PrivateRequest, SessionLease},
        service::Service,
    },
    resource_fs::ScanControl,
};
use ag1_storage::mcp_private::PrivateStore;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn private_manager(path: &Path) -> Arc<Manager> {
    Arc::new(
        Manager::open(
            Store::open(&path.join("public")).unwrap(),
            Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
            DiscoveryLimits::new(Duration::from_secs(2)).unwrap(),
        )
        .unwrap()
        .with_vault(Vault::open(PrivateStore::open(&path.join("private")).unwrap()).unwrap())
        .unwrap(),
    )
}
fn material() -> Material {
    Material::Environment(BTreeMap::from([(
        "TOKEN".into(),
        SecretBytes::new(b"owned-mcp-value".to_vec()),
    )]))
}
fn local_private(path: &Path) -> Draft {
    let mut draft = local(path, "normal", Revision::Current20260728);
    // The credential-aware wrapper checks/removes its synthetic secret before
    // running the unchanged unauthenticated protocol peer's environment checks.
    let wrapper = path.join("private-peer.py");
    std::fs::write(&wrapper,"import os\nfrom pathlib import Path\nassert os.environ.pop('TOKEN') == 'owned-mcp-value'\nPath('private-entry-seen').write_text('received')\nexec(Path('peer.py').read_text())\n").unwrap();
    if let Configuration::Local {
        arguments,
        secret_environment,
        ..
    } = &mut draft.configuration
    {
        arguments[1] = wrapper.to_str().unwrap().into();
        secret_environment.insert("TOKEN".into());
    }
    draft
}
fn revision(m: &Manager, id: &str) -> u64 {
    m.records()
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
        .revision
}
fn save(m: &Manager, id: &str) {
    m.save_private(m.review_private(id, revision(m, id)).unwrap(), material())
        .unwrap();
}
#[test]
fn private_save_is_inert_and_exact_values_only_reach_the_reviewed_child() {
    let dir = tempfile::tempdir().unwrap();
    let m = private_manager(dir.path());
    let id = m
        .add_drafts(vec![local_private(dir.path())])
        .unwrap()
        .remove(0);
    assert!(
        m.review_test_connection(&id, CatalogScope::GeneralChat)
            .is_err()
    );
    let review = m.review_private(&id, 1).unwrap();
    let public = serde_json::to_string(review.request()).unwrap();
    assert!(public.contains("TOKEN"));
    assert!(!public.contains("owned-mcp-value"));
    m.save_private(review, material()).unwrap();
    assert!(!dir.path().join("private-entry-seen").exists());
    assert!(events(dir.path()).is_empty());
    assert!(!m.records().unwrap()[0].availability.enabled);
    let auth = m.authentication(&id).unwrap().unwrap();
    assert_eq!(auth.status, Status::Connected);
    assert!(auth.grant.account.is_none());
    enable(&m, &id);
    let connection = m
        .connect(
            m.review_connection(&id, CatalogScope::GeneralChat).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &connection, ConnectionHealth::Ready);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("private-entry-seen")).unwrap(),
        "received"
    );
    let catalog = m.catalog(&connection, &CatalogScope::GeneralChat).unwrap();
    assert_eq!(catalog.binding.auth_profile, auth.binding.profile_id);
    assert_eq!(catalog.binding.credential_generation, auth.generation);
    assert!(
        !serde_json::to_string(&m.records().unwrap())
            .unwrap()
            .contains("owned-mcp-value")
    );
    m.disconnect(&connection).unwrap();
    m.wait_disconnected(&connection).unwrap();
    drop(m);
    let m = private_manager(dir.path());
    assert_eq!(
        m.authentication(&id).unwrap().unwrap().status,
        Status::Connected
    );
    assert!(m.review_connection(&id, CatalogScope::GeneralChat).is_ok());
}
#[test]
fn logout_retires_all_server_sessions_pending_reviews_and_selection_but_preserves_peer() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let m = private_manager(dir.path());
    let ids = m
        .add_drafts(vec![
            local_private(&a),
            local(&b, "normal", Revision::Current20260728),
        ])
        .unwrap();
    save(&m, &ids[0]);
    for id in &ids {
        enable(&m, id);
    }
    let one = SessionLease::new(m.clone(), CatalogScope::GeneralChat).unwrap();
    let two =
        SessionLease::new(m.clone(), CatalogScope::Workspace("second-project".into())).unwrap();
    let ca = one
        .connect(
            one.review(&ids[0], false).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    let cb = two
        .connect(
            two.review(&ids[0], false).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    let peer = m
        .connect(
            m.review_connection(&ids[1], CatalogScope::GeneralChat)
                .unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    for id in [&ca, &cb, &peer] {
        wait(&m, id, ConnectionHealth::Ready);
    }
    let pending = m
        .review_connection(&ids[0], CatalogScope::GeneralChat)
        .unwrap();
    let mut registry = ag1_core::tools::registry::Registry::builtins().unwrap();
    let selection = one
        .select(
            &mut registry,
            &[ag1_platform::mcp::manager::SelectedCapability {
                connection_id: ca.clone(),
                kind: ag1_core::tools::mcp::Kind::Tool,
                key: "first".into(),
            }],
        )
        .unwrap();
    selection.validate(&m, &registry).unwrap();
    m.logout(&ids[0], revision(&m, &ids[0])).unwrap();
    assert!(one.inspect(&ca).is_err());
    assert!(selection.validate(&m, &registry).is_err());
    assert!(two.inspect(&cb).is_err());
    assert!(
        one.search(ag1_core::tools::mcp::Kind::Tool, "", 20)
            .unwrap()
            .matches
            .is_empty()
    );
    assert!(
        m.connect(pending, ConnectionConsent::LocalHostAccountExecution)
            .is_err()
    );
    assert!(
        m.review_connection(&ids[0], CatalogScope::GeneralChat)
            .is_err()
    );
    assert_eq!(
        m.authentication(&ids[0]).unwrap().unwrap().status,
        Status::Revoked
    );
    assert!(m.catalog(&peer, &CatalogScope::GeneralChat).is_ok());
    one.close().unwrap();
    two.close().unwrap();
    m.disconnect(&peer).unwrap();
    m.wait_disconnected(&peer).unwrap();
}
#[test]
fn changed_configuration_or_retired_session_cannot_save_a_late_private_entry() {
    let dir = tempfile::tempdir().unwrap();
    let m = private_manager(dir.path());
    let id = m
        .add_drafts(vec![local_private(dir.path())])
        .unwrap()
        .remove(0);
    let stale = m.review_private(&id, 1).unwrap();
    let draft = m.records().unwrap()[0].draft.clone();
    m.edit(&id, 1, draft).unwrap();
    assert!(m.save_private(stale, material()).is_err());
    assert!(m.authentication(&id).unwrap().is_none());
    let late = m.review_private(&id, revision(&m, &id)).unwrap();
    m.logout(&id, revision(&m, &id)).unwrap();
    assert!(m.save_private(late, material()).is_err());
    let lease = SessionLease::new(m.clone(), CatalogScope::GeneralChat).unwrap();
    let stale = lease.review_private(&id, revision(&m, &id)).unwrap();
    lease.close().unwrap();
    assert!(m.save_private(stale, material()).is_err());
    save(&m, &id);
    let stale = m.review_private(&id, revision(&m, &id)).unwrap();
    let mut draft = m.records().unwrap()[0].draft.clone();
    draft.name = "Renamed".into();
    m.edit(&id, revision(&m, &id), draft).unwrap();
    assert!(m.save_private(stale, material()).is_err());
    assert!(
        m.review_test_connection(&id, CatalogScope::GeneralChat)
            .is_err()
    );
}
struct Entry {
    seen: AtomicUsize,
}
impl PrivateEntry for Entry {
    fn enter(
        &self,
        request: &PrivateRequest,
        control: &ScanControl<'_>,
    ) -> Result<Material, DomainError> {
        control.check()?;
        assert_eq!(request.server_name, "owned local");
        self.seen.fetch_add(1, Ordering::AcqRel);
        Ok(material())
    }
}
#[test]
fn manager_bridge_requests_native_entry_and_rejects_renderer_secret_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let m = private_manager(dir.path());
    let id = m
        .add_drafts(vec![local_private(dir.path())])
        .unwrap()
        .remove(0);
    let service = Service::from_manager(
        m.clone(),
        Path::new(env!("CARGO_BIN_EXE_ag1-process-host")).into(),
    );
    let entry = Arc::new(Entry {
        seen: AtomicUsize::new(0),
    });
    service.set_private_entry(entry.clone()).unwrap();
    let scope = Scope {
        workspace: "native-workspace".into(),
        session: "native-session".into(),
        epoch: 1,
        revision: 0,
    };
    let session = service
        .session(CatalogScope::GeneralChat, scope.clone(), None)
        .unwrap();
    assert!(session.dispatch(&scope,&serde_json::json!({"action":"private_entry","id":id,"revision":1,"token":"renderer-value"})).is_err());
    assert_eq!(entry.seen.load(Ordering::Acquire), 0);
    session
        .dispatch(
            &scope,
            &serde_json::json!({"action":"private_entry","id":id,"revision":1}),
        )
        .unwrap();
    assert_eq!(entry.seen.load(Ordering::Acquire), 1);
    let public = session
        .dispatch(&scope, &serde_json::json!({"action":"list"}))
        .unwrap();
    assert_eq!(public["privateEntryAvailable"], true);
    assert_eq!(
        public["servers"][0]["authentication"]["status"],
        "connected"
    );
    assert!(!public.to_string().contains("owned-mcp-value"));
    assert!(events(dir.path()).is_empty());
    session.close().unwrap();
    assert!(
        session
            .dispatch(
                &scope,
                &serde_json::json!({"action":"private_entry","id":id,"revision":revision(&m,&id)})
            )
            .is_err()
    );
    assert_eq!(entry.seen.load(Ordering::Acquire), 1);
}

#[test]
fn supplied_secret_echo_in_catalog_metadata_is_never_published() {
    let dir = tempfile::tempdir().unwrap();
    let m = private_manager(dir.path());
    let draft = local_private(dir.path());
    let wrapper = dir.path().join("private-peer.py");
    std::fs::write(&wrapper,"import os\nfrom pathlib import Path\nsecret=os.environ.pop('TOKEN')\nassert secret=='owned-mcp-value'\ncode=Path('peer.py').read_text().replace('owned-fixture',secret)\nexec(code)\n").unwrap();
    let id = m.add_drafts(vec![draft]).unwrap().remove(0);
    save(&m, &id);
    enable(&m, &id);
    let connection = m
        .connect(
            m.review_connection(&id, CatalogScope::GeneralChat).unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    wait(&m, &connection, ConnectionHealth::Failed);
    assert_eq!(
        m.status(&connection).unwrap().diagnostic,
        Some(ag1_platform::mcp::manager::Diagnostic::PrivateValueEcho)
    );
    assert!(
        m.inspect_catalog(&connection, &CatalogScope::GeneralChat)
            .is_err()
    );
    assert!(
        m.search_tools(&CatalogScope::GeneralChat, "", 20)
            .unwrap()
            .matches
            .is_empty()
    );
    m.wait_disconnected(&connection).unwrap();
}

#[test]
fn replacing_the_reviewed_working_directory_never_injects_old_private_values() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let m = private_manager(dir.path());
    let id = m.add_drafts(vec![local_private(&work)]).unwrap().remove(0);
    save(&m, &id);
    let old = dir.path().join("old-work");
    std::fs::rename(&work, &old).unwrap();
    std::fs::create_dir(&work).unwrap();
    for file in ["peer.py", "private-peer.py"] {
        std::fs::copy(old.join(file), work.join(file)).unwrap();
    }
    assert!(
        m.review_test_connection(&id, CatalogScope::GeneralChat)
            .is_err()
    );
    assert!(!work.join("private-entry-seen").exists());
    assert!(events(&work).is_empty());
}

#[test]
fn private_environment_accounts_for_shared_process_packet_limit_before_saving() {
    let dir = tempfile::tempdir().unwrap();
    let m = private_manager(dir.path());
    let mut draft = local_private(dir.path());
    let names: BTreeSet<String> = (0..20).map(|n| format!("TOKEN_{n}")).collect();
    if let Configuration::Local {
        secret_environment, ..
    } = &mut draft.configuration
    {
        *secret_environment = names.clone();
    }
    let id = m.add_drafts(vec![draft]).unwrap().remove(0);
    let material = Material::Environment(
        names
            .into_iter()
            .map(|n| (n, SecretBytes::new(vec![b'x'; 4096])))
            .collect(),
    );
    assert!(matches!(
        m.save_private(m.review_private(&id, 1).unwrap(), material),
        Err(DomainError::LimitExceeded(_))
    ));
    assert_eq!(revision(&m, &id), 1);
    assert!(m.authentication(&id).unwrap().is_none());
    assert!(events(dir.path()).is_empty());
}

#[test]
fn oauth_service_accepts_only_public_action_ids_and_requires_native_port_and_session_owner() {
    use ag1_platform::mcp::manager::{OAuthEntry, OAuthReview};
    struct NativeOAuth(AtomicUsize);
    impl OAuthEntry for NativeOAuth {
        fn enter(
            &self,
            session: &SessionLease,
            review: OAuthReview,
            _refresh: bool,
            control: &ScanControl<'_>,
        ) -> Result<(), DomainError> {
            session.check()?;
            control.check()?;
            assert!(matches!(
                review.request().configuration,
                Configuration::Http { .. }
            ));
            self.0.fetch_add(1, Ordering::AcqRel);
            Err(DomainError::Cancelled)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let m = private_manager(dir.path());
    let id = m
        .add_drafts(vec![remote("owned OAuth fixture")])
        .unwrap()
        .remove(0);
    let service = Service::from_manager(
        m.clone(),
        Path::new(env!("CARGO_BIN_EXE_ag1-process-host")).into(),
    );
    let scope = Scope {
        workspace: "owned-workspace".into(),
        session: "owned-session".into(),
        epoch: 1,
        revision: 0,
    };
    let session = service
        .session(CatalogScope::GeneralChat, scope.clone(), None)
        .unwrap();
    let command = serde_json::json!({"action":"oauth","id":id,"revision":1,"refresh":false});
    assert!(matches!(
        session.dispatch(&scope, &command),
        Err(DomainError::Unavailable(_))
    ));
    let entry = Arc::new(NativeOAuth(AtomicUsize::new(0)));
    service.set_oauth_entry(entry.clone()).unwrap();
    for key in [
        "code",
        "token",
        "refresh_token",
        "issuer",
        "client_id",
        "authorization_url",
    ] {
        let mut bad = command.clone();
        bad[key] = serde_json::json!("renderer-value");
        assert!(session.dispatch(&scope, &bad).is_err());
    }
    let foreign = Scope {
        session: "foreign".into(),
        ..scope.clone()
    };
    assert!(session.dispatch(&foreign, &command).is_err());
    assert_eq!(entry.0.load(Ordering::Acquire), 0);
    assert_eq!(
        session.dispatch(&scope, &command).err(),
        Some(DomainError::Cancelled)
    );
    assert_eq!(entry.0.load(Ordering::Acquire), 1);
    let listing = session
        .dispatch(&scope, &serde_json::json!({"action":"list"}))
        .unwrap();
    assert_eq!(listing["oauthAvailable"], true);
    assert!(m.authentication(&id).unwrap().is_none());
    assert!(!m.records().unwrap()[0].availability.enabled);
    session.close().unwrap();
    assert!(session.dispatch(&scope, &command).is_err());
    assert_eq!(entry.0.load(Ordering::Acquire), 1);
}
