use super::*;
use ag1_core::SecretBytes;
use ag1_platform::mcp::auth::{Binding, Grant, Material, Target, Vault};
use ag1_storage::mcp_private::PrivateStore;
const PROFILE: &str = "20000000-0000-4000-8000-000000000002";
// The frozen unauthenticated peer asserts that Authorization is absent. Use a
// separate credential-aware peer; keep every original assertion unchanged.
struct Fixture {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<()>>>,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let copy = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let mode = mode.to_owned();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let mut socket = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                socket.set_read_timeout(Some(Duration::from_secs(2)))?;
                socket.set_write_timeout(Some(Duration::from_secs(2)))?;
                let mut raw = Vec::new();
                while !raw.ends_with(b"\r\n\r\n") {
                    if raw.len() >= 8192 {
                        return Err(std::io::Error::other("fixture header bounds"));
                    }
                    let mut byte = [0];
                    socket.read_exact(&mut byte)?;
                    raw.push(byte[0]);
                }
                let raw = std::str::from_utf8(&raw).map_err(std::io::Error::other)?;
                let mut lines = raw.split("\r\n");
                let verb = lines
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_owned();
                let headers: BTreeMap<String, String> = lines
                    .filter_map(|s| s.split_once(':'))
                    .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().into()))
                    .collect();
                let n: usize = headers
                    .get("content-length")
                    .map(|s| s.parse().unwrap())
                    .unwrap_or(0);
                if n > 8192 {
                    return Err(std::io::Error::other("fixture body bounds"));
                }
                let mut bytes = vec![0; n];
                socket.read_exact(&mut bytes)?;
                let body: Value = if bytes.is_empty() {
                    json!({})
                } else {
                    serde_json::from_slice(&bytes).map_err(std::io::Error::other)?
                };
                copy.lock().unwrap().push(Seen {
                    verb: verb.clone(),
                    headers: headers.clone(),
                    body: body.clone(),
                });
                if headers.get("authorization").map(String::as_str)
                    != Some("Bearer owned-http-token")
                    || headers.contains_key("cookie")
                    || headers.contains_key("proxy-authorization")
                {
                    respond(&mut socket, 401, "application/json", "", b"{}")?;
                    continue;
                }
                if let Some(target) = mode.strip_prefix("redirect:") {
                    respond(
                        &mut socket,
                        307,
                        "application/json",
                        &format!("Location: {target}\r\n"),
                        b"",
                    )?;
                    continue;
                }
                let method = body["method"].as_str().unwrap_or("");
                if verb == "DELETE" || method == "notifications/initialized" {
                    respond(&mut socket, 202, "application/json", "", b"")?;
                    continue;
                }
                let result = match method {
                    "initialize" => {
                        json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{},"resources":{},"prompts":{}},"serverInfo":{"name":"owned-auth-fixture","version":"1"}})
                    }
                    "server/discover" => {
                        json!({"resultType":"complete","supportedVersions":["2026-07-28"],"ttlMs":0,"cacheScope":"private","capabilities":{"tools":{},"resources":{},"prompts":{}}})
                    }
                    "tools/list" => json!({"tools":[]}),
                    "resources/list" => json!({"resources":[]}),
                    "resources/templates/list" => json!({"resourceTemplates":[]}),
                    "prompts/list" => json!({"prompts":[]}),
                    _ => return Err(std::io::Error::other("unexpected fixture operation")),
                };
                let extra = if method == "initialize" {
                    "Mcp-Session-Id: owned-auth-session\r\n"
                } else {
                    ""
                };
                respond(
                    &mut socket,
                    200,
                    "application/json",
                    extra,
                    &serde_json::to_vec(&json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
                        .unwrap(),
                )?;
            }
            Ok(())
        });
        Self {
            url,
            seen,
            stop,
            worker: Some(worker),
        }
    }
    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap().unwrap();
        }
    }
}
struct Auth {
    _directory: tempfile::TempDir,
    vault: Arc<Vault>,
    target: Binding,
    generation: u64,
}
impl Auth {
    fn new(fixture: &Fixture) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::open(PrivateStore::open(directory.path()).unwrap()).unwrap();
        let target = Binding {
            server_id: binding(Revision::Current20260728).server_id,
            profile_id: PROFILE.into(),
            configuration_revision: 1,
            configuration_sha256: "a".repeat(64),
            target: Target::ManualBearer {
                endpoint: fixture.url.clone(),
                loopback_development: true,
            },
        };
        let generation = vault
            .commit(
                vault.begin_login(target.clone()).unwrap(),
                Material::Bearer {
                    access: SecretBytes::new(b"owned-http-token".to_vec()),
                    refresh: None,
                },
                Grant::default(),
                1,
            )
            .unwrap();
        Self {
            _directory: directory,
            vault,
            target,
            generation,
        }
    }
    fn binding(&self, revision: Revision) -> CatalogBinding {
        CatalogBinding {
            auth_profile: PROFILE.into(),
            credential_generation: self.generation,
            ..binding(revision)
        }
    }
    fn client(&self, fixture: &Fixture, revision: Revision) -> HttpDiscovery {
        HttpDiscovery::new(
            Endpoint::loopback_development(&fixture.url).unwrap(),
            self.binding(revision),
            DiscoveryLimits::new(Duration::from_secs(3)).unwrap(),
        )
        .unwrap()
        .with_credential(Arc::new(
            self.vault.lease(&self.target, self.generation, 1).unwrap(),
        ))
        .unwrap()
    }
}
#[tokio::test]
async fn native_bearer_is_sent_on_every_discovery_and_shutdown_request_for_both_revisions() {
    for revision in [Revision::Current20260728, Revision::Legacy20251125] {
        let fixture = Fixture::new("json");
        let auth = Auth::new(&fixture);
        let mut client = auth.client(&fixture, revision);
        let (_source, mut cancel) = cancellation();
        client.negotiate(&mut cancel).await.unwrap();
        client.refresh_tools(&mut cancel).await.unwrap();
        for kind in [
            CatalogKind::Resources,
            CatalogKind::ResourceTemplates,
            CatalogKind::Prompts,
        ] {
            client.list(kind, &mut cancel).await.unwrap();
        }
        client.shutdown(&mut cancel).await.unwrap();
        let requests = fixture.requests();
        assert!(requests.len() >= 5);
        for r in requests {
            assert_eq!(
                r.headers.get("authorization").map(String::as_str),
                Some("Bearer owned-http-token")
            );
            assert!(!r.body.to_string().contains("owned-http-token"));
        }
    }
}
#[tokio::test]
async fn logout_and_missing_credential_prevent_new_requests_without_unauthenticated_fallback() {
    let fixture = Fixture::new("json");
    let auth = Auth::new(&fixture);
    let mut client = auth.client(&fixture, Revision::Current20260728);
    let (_source, mut cancel) = cancellation();
    client.negotiate(&mut cancel).await.unwrap();
    let count = fixture.requests().len();
    auth.vault.logout(&auth.target).unwrap();
    assert!(client.refresh_tools(&mut cancel).await.is_err());
    assert_eq!(fixture.requests().len(), count);
    let mut missing = HttpDiscovery::new(
        Endpoint::loopback_development(&fixture.url).unwrap(),
        auth.binding(Revision::Current20260728),
        DiscoveryLimits::new(Duration::from_secs(3)).unwrap(),
    )
    .unwrap();
    assert!(missing.negotiate(&mut cancel).await.is_err());
    assert_eq!(fixture.requests().len(), count);
}
#[tokio::test]
async fn credential_cannot_cross_endpoint_profile_generation_or_redirect() {
    let other = Fixture::new("json");
    let source = Fixture::new(&format!("redirect:{}", other.url));
    let auth = Auth::new(&source);
    let mut client = auth.client(&source, Revision::Current20260728);
    let (_source, mut cancel) = cancellation();
    assert!(client.negotiate(&mut cancel).await.is_err());
    assert!(other.requests().is_empty());
    assert_eq!(source.requests().len(), 1);
    assert_eq!(
        source.requests()[0].headers["authorization"],
        "Bearer owned-http-token"
    );
    for (url, binding) in [
        (other.url.clone(), auth.binding(Revision::Current20260728)),
        (
            source.url.clone(),
            CatalogBinding {
                auth_profile: "30000000-0000-4000-8000-000000000003".into(),
                ..auth.binding(Revision::Current20260728)
            },
        ),
        (
            source.url.clone(),
            CatalogBinding {
                credential_generation: auth.generation + 1,
                ..auth.binding(Revision::Current20260728)
            },
        ),
    ] {
        let client = HttpDiscovery::new(
            Endpoint::loopback_development(&url).unwrap(),
            binding,
            DiscoveryLimits::new(Duration::from_secs(3)).unwrap(),
        )
        .unwrap();
        assert!(
            client
                .with_credential(Arc::new(
                    auth.vault.lease(&auth.target, auth.generation, 1).unwrap()
                ))
                .is_err()
        );
    }
    assert!(other.requests().is_empty());
}

#[test]
fn actual_manager_composition_connects_with_saved_remote_token_and_retires_on_logout() {
    use ag1_platform::mcp::{
        definitions::{Availability, Configuration, Draft, Provenance},
        manager::{ConnectionConsent, ConnectionHealth, Manager},
    };
    let fixture = Fixture::new("json");
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(PrivateStore::open(&dir.path().join("private")).unwrap()).unwrap();
    let manager = Manager::open(
        ag1_storage::Store::open(&dir.path().join("public")).unwrap(),
        std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
        DiscoveryLimits::new(Duration::from_secs(3)).unwrap(),
    )
    .unwrap()
    .with_vault(vault)
    .unwrap();
    let id = manager
        .add_drafts(vec![Draft {
            name: "owned remote".into(),
            configuration: Configuration::Http {
                url: fixture.url.clone(),
                loopback_development: true,
            },
            protocol: Revision::Current20260728.as_str().into(),
            auth_profile: None,
            provenance: Provenance::Manual,
        }])
        .unwrap()
        .remove(0);
    manager
        .save_private(
            manager.review_private(&id, 1).unwrap(),
            Material::Bearer {
                access: SecretBytes::new(b"owned-http-token".to_vec()),
                refresh: None,
            },
        )
        .unwrap();
    assert!(fixture.requests().is_empty());
    manager
        .set_availability(
            &id,
            2,
            Availability {
                enabled: true,
                general_chat: true,
                ..Default::default()
            },
        )
        .unwrap();
    let connection = manager
        .connect(
            manager
                .review_connection(&id, CatalogScope::GeneralChat)
                .unwrap(),
            ConnectionConsent::LoopbackDevelopmentEndpoint,
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while manager.status(&connection).unwrap().health != ConnectionHealth::Ready {
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let catalog = manager
        .catalog(&connection, &CatalogScope::GeneralChat)
        .unwrap();
    assert!(catalog.binding.credential_generation > 0);
    assert_eq!(
        catalog.binding.auth_profile,
        manager.records().unwrap()[0]
            .draft
            .auth_profile
            .as_deref()
            .unwrap()
    );
    for request in fixture.requests() {
        assert_eq!(request.headers["authorization"], "Bearer owned-http-token");
    }
    manager.logout(&id, 3).unwrap();
    assert!(
        manager
            .inspect_catalog(&connection, &CatalogScope::GeneralChat)
            .is_err()
    );
    manager.wait_disconnected(&connection).unwrap();
}
