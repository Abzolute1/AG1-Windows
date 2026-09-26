use ag1_core::{DomainError, SecretBytes};
use ag1_platform::mcp::auth::{Binding, Grant, Material, Status, Target, Vault};
use ag1_storage::mcp_private::PrivateStore;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
const SERVER: &str = "10000000-0000-4000-8000-000000000001";
const PROFILE: &str = "20000000-0000-4000-8000-000000000002";
fn binding() -> Binding {
    Binding {
        server_id: SERVER.into(),
        profile_id: PROFILE.into(),
        configuration_revision: 1,
        configuration_sha256: "a".repeat(64),
        target: Target::ManualBearer {
            endpoint: "https://example.invalid/mcp".into(),
            loopback_development: false,
        },
    }
}
fn bearer(s: &str) -> Material {
    Material::Bearer {
        access: SecretBytes::new(s.as_bytes().to_vec()),
        refresh: None,
    }
}
fn oauth() -> Binding {
    Binding {
        target: Target::OAuth {
            endpoint: "https://example.invalid/mcp".into(),
            resource: "https://example.invalid/mcp".into(),
            issuer: "https://issuer.invalid/auth".into(),
            token_endpoint: "https://issuer.invalid/token".into(),
            client_id: "owned-client".into(),
        },
        ..binding()
    }
}
fn oauth_tokens() -> Material {
    Material::Bearer {
        access: SecretBytes::new(b"owned-access".to_vec()),
        refresh: Some(SecretBytes::new(b"owned-refresh".to_vec())),
    }
}
fn grant() -> Grant {
    Grant {
        account: Some("verified-subject".into()),
        scopes: BTreeSet::from(["files:read".into()]),
        expires_at_ms: Some(1000),
    }
}
fn vault(dir: &tempfile::TempDir) -> Arc<Vault> {
    Vault::open(PrivateStore::open(dir.path()).unwrap()).unwrap()
}

#[test]
fn native_lease_rechecks_exact_identity_expiry_and_logout_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let b = binding();
    let pending = v.begin_login(b.clone()).unwrap();
    assert_eq!(v.records(1).unwrap()[0].status, Status::Authenticating);
    let generation = v
        .commit(
            pending,
            bearer("owned-token"),
            Grant {
                expires_at_ms: Some(100),
                ..Default::default()
            },
            1,
        )
        .unwrap();
    let lease = v.lease(&b, generation, 1).unwrap();
    lease
        .with_material(1, |m| {
            assert!(matches!(m,Material::Bearer {access,..} if access.expose()==b"owned-token"));
            Ok(())
        })
        .unwrap();
    assert!(lease.check(100).is_err());
    assert_eq!(v.records(100).unwrap()[0].status, Status::Expired);
    for foreign in [
        Binding {
            server_id: "30000000-0000-4000-8000-000000000003".into(),
            ..b.clone()
        },
        Binding {
            profile_id: "30000000-0000-4000-8000-000000000003".into(),
            ..b.clone()
        },
        Binding {
            configuration_revision: 2,
            ..b.clone()
        },
        Binding {
            configuration_sha256: "b".repeat(64),
            ..b.clone()
        },
        Binding {
            target: Target::ManualBearer {
                endpoint: "https://example.invalid/other".into(),
                loopback_development: false,
            },
            ..b.clone()
        },
    ] {
        assert!(v.lease(&foreign, generation, 1).is_err());
    }
    assert!(v.lease(&b, generation + 1, 1).is_err());
    let public = serde_json::to_string(&v.records(1).unwrap()).unwrap();
    assert!(!public.contains("owned-token"));
    assert!(public.contains("example.invalid/mcp"));
    drop(lease);
    drop(v);
    let v = vault(&dir);
    let lease = v.lease(&b, generation, 1).unwrap();
    v.logout(&b).unwrap();
    assert!(lease.check(1).is_err());
    drop(lease);
    drop(v);
    let v = vault(&dir);
    assert_eq!(v.records(1).unwrap()[0].status, Status::Revoked);
    assert!(v.lease(&b, generation, 1).is_err());
    assert_eq!(
        std::fs::read_dir(dir.path().join("blobs")).unwrap().count(),
        0
    );
}
#[test]
fn logout_fences_inflight_login_refresh_and_foreign_vault_operations() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let foreign = vault(&other);
    let b = oauth();
    let late = v.begin_login(b.clone()).unwrap();
    v.logout(&b).unwrap();
    assert!(v.commit(late, oauth_tokens(), grant(), 1).is_err());
    let pending = v.begin_login(b.clone()).unwrap();
    assert!(foreign.commit(pending, oauth_tokens(), grant(), 1).is_err());
    let generation = v
        .commit(
            v.begin_login(b.clone()).unwrap(),
            oauth_tokens(),
            grant(),
            1,
        )
        .unwrap();
    let lease = v.lease(&b, generation, 1).unwrap();
    let late = v.begin_refresh(&b, generation).unwrap();
    assert_eq!(late.refresh_token().unwrap().expose(), b"owned-refresh");
    assert!(v.begin_refresh(&b, generation).is_err());
    v.logout(&b).unwrap();
    assert!(v.commit(late, oauth_tokens(), grant(), 1).is_err());
    assert!(lease.check(1).is_err());
    assert_eq!(v.records(1).unwrap()[0].status, Status::Revoked);
}
#[test]
fn refresh_is_shared_serialized_and_cannot_change_account_or_scope_silently() {
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let b = oauth();
    let generation = v
        .commit(
            v.begin_login(b.clone()).unwrap(),
            oauth_tokens(),
            grant(),
            1,
        )
        .unwrap();
    let old = v.lease(&b, generation, 1).unwrap();
    let pending = v.begin_refresh(&b, generation).unwrap();
    let peer = v.clone();
    let target = b.clone();
    assert!(
        std::thread::spawn(move || peer.begin_refresh(&target, generation).is_err())
            .join()
            .unwrap()
    );
    drop(pending); // Cancelled network worker releases its reservation.
    let changed = Grant {
        account: Some("other-account".into()),
        ..grant()
    };
    assert!(
        v.commit(
            v.begin_refresh(&b, generation).unwrap(),
            oauth_tokens(),
            changed,
            1
        )
        .is_err()
    );
    let changed = Grant {
        scopes: BTreeSet::from(["admin".into()]),
        ..grant()
    };
    assert!(
        v.commit(
            v.begin_refresh(&b, generation).unwrap(),
            oauth_tokens(),
            changed,
            1
        )
        .is_err()
    );
    old.check(1).unwrap();
    let newer = v
        .commit(
            v.begin_refresh(&b, generation).unwrap(),
            oauth_tokens(),
            Grant {
                expires_at_ms: Some(2000),
                ..grant()
            },
            1,
        )
        .unwrap();
    assert!(newer > generation);
    assert!(old.check(1).is_err());
    v.lease(&b, newer, 1500).unwrap();
}
#[test]
fn restart_does_not_resume_browser_login_or_reuse_old_callback_owner() {
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let b = binding();
    let pending = v.begin_login(b.clone()).unwrap();
    drop(v);
    let v = vault(&dir);
    assert_eq!(v.records(1).unwrap()[0].status, Status::Revoked);
    assert!(
        v.commit(pending, bearer("late-login"), Grant::default(), 1)
            .is_err()
    );
    let generation = v
        .commit(
            v.begin_login(b.clone()).unwrap(),
            bearer("new-login"),
            Grant::default(),
            1,
        )
        .unwrap();
    v.lease(&b, generation, 1).unwrap();
}
#[test]
fn local_private_values_require_exact_reviewed_names_and_never_become_bearer() {
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let b = Binding {
        target: Target::Local {
            executable_sha256: "c".repeat(64),
            environment_names: BTreeSet::from(["API_TOKEN".into(), "PASSWORD".into()]),
        },
        ..binding()
    };
    let wrong = Material::Environment(BTreeMap::from([(
        "API_TOKEN".into(),
        SecretBytes::new(b"private".to_vec()),
    )]));
    assert!(
        v.commit(
            v.begin_login(b.clone()).unwrap(),
            wrong,
            Grant::default(),
            1
        )
        .is_err()
    );
    assert!(
        v.commit(
            v.begin_login(b.clone()).unwrap(),
            bearer("private"),
            Grant::default(),
            1
        )
        .is_err()
    );
    let material = Material::Environment(BTreeMap::from([
        (
            "API_TOKEN".into(),
            SecretBytes::new(b"private-value".to_vec()),
        ),
        (
            "PASSWORD".into(),
            SecretBytes::new(b"other-private".to_vec()),
        ),
    ]));
    let generation = v
        .commit(
            v.begin_login(b.clone()).unwrap(),
            material,
            Grant::default(),
            1,
        )
        .unwrap();
    drop(v);
    let v = vault(&dir);
    let lease = v.lease(&b, generation, 1).unwrap();
    lease.with_material(1,|m|{assert!(matches!(m,Material::Environment(values) if values["API_TOKEN"].expose()==b"private-value"));Ok(())}).unwrap();
    let public = serde_json::to_string(&v.records(1).unwrap()).unwrap();
    assert!(public.contains("API_TOKEN"));
    assert!(!public.contains("private-value"));
}
#[test]
fn credentials_do_not_fall_back_to_provider_store_and_reject_header_injection() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = ag1_storage::Store::open(dir.path()).unwrap();
        store
            .save_metadata(
                "provider-credential",
                PROFILE,
                None,
                br#"{"key":"provider-token"}"#,
            )
            .unwrap();
    }
    let v = vault(&dir);
    assert!(v.records(1).unwrap().is_empty());
    assert!(v.lease(&binding(), 1, 1).is_err());
    for value in [
        "bad\r\nX-Header: data",
        "has space",
        "",
        "=leading",
        "token=tail",
    ] {
        assert!(
            v.commit(
                v.begin_login(binding()).unwrap(),
                bearer(value),
                Grant::default(),
                1
            )
            .is_err()
        );
    }
    let bad = Binding {
        target: Target::ManualBearer {
            endpoint: "https://example.invalid/mcp?token=private".into(),
            loopback_development: false,
        },
        ..binding()
    };
    assert!(bad.validate().is_err());
    let bad = Binding {
        target: Target::ManualBearer {
            endpoint: "http://127.0.0.1:1234/mcp".into(),
            loopback_development: false,
        },
        ..binding()
    };
    assert!(matches!(bad.validate(), Err(DomainError::PermissionDenied)));
}

#[test]
fn abandoned_login_is_visible_as_revoked_and_a_new_attempt_retires_old_admissions() {
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let b = binding();
    let pending = v.begin_login(b.clone()).unwrap();
    drop(pending);
    assert_eq!(v.records(1).unwrap()[0].status, Status::Revoked);
    let generation = v
        .commit(
            v.begin_login(b.clone()).unwrap(),
            bearer("old"),
            Grant::default(),
            1,
        )
        .unwrap();
    let old = v.lease(&b, generation, 1).unwrap();
    let stale = v.begin_login(b.clone()).unwrap();
    assert!(old.check(1).is_err());
    let current = v.begin_login(b.clone()).unwrap();
    assert!(
        v.commit(stale, bearer("stale"), Grant::default(), 1)
            .is_err()
    );
    let newer = v
        .commit(current, bearer("current"), Grant::default(), 1)
        .unwrap();
    assert!(newer > generation);
    v.lease(&b, newer, 1).unwrap();
}

#[test]
fn uncertain_logout_poisoning_revokes_live_access_and_reopen_observes_durable_tombstone() {
    use ag1_core::Clock;
    use ag1_storage::{Boundary, FaultInjector, StorageError, StorageResult, Store};
    use std::sync::atomic::{AtomicBool, Ordering};
    struct TestClock;
    impl Clock for TestClock {
        fn now_ms(&self) -> u64 {
            1
        }
    }
    struct Fault(AtomicBool);
    impl FaultInjector for Fault {
        fn check(&self, b: Boundary) -> StorageResult<()> {
            if b == Boundary::BeforeAck && self.0.swap(false, Ordering::AcqRel) {
                Err(StorageError::Injected(b))
            } else {
                Ok(())
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let fault = Arc::new(Fault(AtomicBool::new(false)));
    let store = Store::open_with(dir.path(), Arc::new(TestClock), fault.clone()).unwrap();
    let v = Vault::open(PrivateStore::from_store(store).unwrap()).unwrap();
    let b = binding();
    let generation = v
        .commit(
            v.begin_login(b.clone()).unwrap(),
            bearer("private"),
            Grant::default(),
            1,
        )
        .unwrap();
    let lease = v.lease(&b, generation, 1).unwrap();
    fault.0.store(true, Ordering::Release);
    assert!(v.logout(&b).is_err());
    assert!(lease.check(1).is_err());
    assert!(v.records(1).is_err());
    drop(lease);
    drop(v);
    let v = vault(&dir);
    assert_eq!(v.records(1).unwrap()[0].status, Status::Revoked);
    assert!(v.lease(&b, generation, 1).is_err());
}

#[test]
fn private_envelope_rejects_oversized_counts_unknown_versions_and_corrupt_headers() {
    let cases = [
        b"unknown-private-version".to_vec(),
        [b"AG1-MCP-PRIVATE-1\0".as_slice(), &129u32.to_le_bytes()].concat(),
        [
            b"AG1-MCP-PRIVATE-1\0".as_slice(),
            &1u32.to_le_bytes(),
            &u32::MAX.to_le_bytes(),
        ]
        .concat(),
    ];
    for raw in cases {
        let dir = tempfile::tempdir().unwrap();
        let store = PrivateStore::open(dir.path()).unwrap();
        store.commit(0, &SecretBytes::new(raw)).unwrap();
        assert!(Vault::open(store).is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let store = PrivateStore::open(dir.path()).unwrap();
    let header = b"{\"generation\":1,\"generation\":2}";
    let raw = [
        b"AG1-MCP-PRIVATE-1\0".as_slice(),
        &1u32.to_le_bytes(),
        &(header.len() as u32).to_le_bytes(),
        header,
    ]
    .concat();
    store.commit(0, &SecretBytes::new(raw)).unwrap();
    assert!(Vault::open(store).is_err());
}

#[test]
fn aggregate_private_quota_rejects_new_payload_without_poisoning_existing_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let mut leases = Vec::new();
    let mut rejected = false;
    let names: BTreeSet<String> = (0..32).map(|n| format!("TOKEN_{n:02}")).collect();
    for n in 0..8 {
        let b = Binding {
            profile_id: format!("20000000-0000-4000-8000-{n:012x}"),
            target: Target::Local {
                executable_sha256: "c".repeat(64),
                environment_names: names.clone(),
            },
            ..binding()
        };
        let pending = v.begin_login(b.clone()).unwrap();
        let material = Material::Environment(
            names
                .iter()
                .map(|name| (name.clone(), SecretBytes::new(vec![b's'; 4096])))
                .collect(),
        );
        match v.commit(pending, material, Grant::default(), 1) {
            Ok(generation) => leases.push(v.lease(&b, generation, 1).unwrap()),
            Err(DomainError::LimitExceeded(_)) => {
                rejected = true;
                break;
            }
            Err(error) => panic!("unexpected quota result: {error:?}"),
        }
    }
    assert!(rejected);
    assert!(!leases.is_empty());
    for lease in &leases {
        lease.check(1).unwrap();
    }
    assert!(v.records(1).is_ok());
}

#[test]
fn oauth_root_resource_never_authorizes_sibling_mcp_endpoints_or_changed_registration() {
    use ag1_platform::mcp::{
        Revision,
        catalog::{CatalogBinding, CatalogScope},
        discovery::DiscoveryLimits,
        http::{Endpoint, HttpDiscovery},
    };
    let dir = tempfile::tempdir().unwrap();
    let v = vault(&dir);
    let mut b = oauth();
    if let Target::OAuth { resource, .. } = &mut b.target {
        *resource = "https://example.invalid".into();
    }
    let mut g = grant();
    g.expires_at_ms = None;
    let generation = v
        .commit(v.begin_login(b.clone()).unwrap(), oauth_tokens(), g, 1)
        .unwrap();
    let native = CatalogBinding {
        server_id: b.server_id.clone(),
        scope: CatalogScope::GeneralChat,
        config_revision: b.configuration_revision,
        auth_profile: b.profile_id.clone(),
        credential_generation: generation,
        connection_generation: 1,
        protocol: Revision::Legacy20251125,
    };
    for endpoint in [
        "https://example.invalid/mcp",
        "https://example.invalid/sibling",
        "https://example.invalid",
    ] {
        let lease = Arc::new(v.lease(&b, generation, 1).unwrap());
        let transport = HttpDiscovery::new(
            Endpoint::public_https(endpoint).unwrap(),
            native.clone(),
            DiscoveryLimits::default(),
        )
        .unwrap();
        assert_eq!(
            transport.with_credential(lease).is_ok(),
            endpoint == "https://example.invalid/mcp"
        );
    }
    for field in 0..5 {
        let mut foreign = b.clone();
        if let Target::OAuth {
            endpoint,
            resource,
            issuer,
            token_endpoint,
            client_id,
        } = &mut foreign.target
        {
            match field {
                0 => *endpoint = "https://example.invalid/sibling".into(),
                1 => *resource = "https://example.invalid/mcp".into(),
                2 => *issuer = "https://foreign.invalid/issuer".into(),
                3 => *token_endpoint = "https://foreign.invalid/token".into(),
                _ => *client_id = "other-client".into(),
            }
        }
        assert!(v.begin_refresh(&foreign, generation).is_err());
        assert!(v.lease(&foreign, generation, 1).is_err());
    }
}
