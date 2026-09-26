//! Owned TLS metadata and native loopback callback fixtures; no live accounts.
use ag1_core::{DomainError, PortFuture};
use ag1_platform::{
    mcp::oauth::{Network, challenge},
    provider::http::cancellation,
    web_http::{SystemResolver, WebPolicy, WebResolver},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{IpAddr, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Reply {
    status: u16,
    headers: Vec<String>,
    body: Vec<u8>,
}
impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            headers: vec!["Content-Type: application/json".into()],
            body: serde_json::to_vec(&value).unwrap(),
        }
    }
    fn code(status: u16) -> Self {
        Self {
            status,
            headers: vec![],
            body: vec![],
        }
    }
}
struct Peer {
    origin: String,
    pem: Vec<u8>,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    _dir: tempfile::TempDir,
    // Network's four-request budget is process-wide. Independent fixture tests
    // must not consume one another's admission slots under the test harness.
    _exclusive: std::sync::MutexGuard<'static, ()>,
}
static FIXTURE: Mutex<()> = Mutex::new(());
impl Peer {
    fn new(routes: impl FnOnce(&str) -> BTreeMap<String, Reply>) -> Self {
        let exclusive = FIXTURE.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key.pem");
        let cert = dir.path().join("cert.pem");
        assert!(
            std::process::Command::new("/usr/bin/openssl")
                .args([
                    "req",
                    "-x509",
                    "-newkey",
                    "rsa:2048",
                    "-nodes",
                    "-days",
                    "1",
                    "-subj",
                    "/CN=localhost",
                    "-addext",
                    "subjectAltName=IP:127.0.0.1,DNS:owned.invalid",
                    "-keyout"
                ])
                .arg(&key)
                .arg("-out")
                .arg(&cert)
                .output()
                .unwrap()
                .status
                .success()
        );
        let pem = std::fs::read(cert).unwrap();
        let acceptor = native_tls::TlsAcceptor::new(
            native_tls::Identity::from_pkcs8(&pem, &std::fs::read(key).unwrap()).unwrap(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("https://{}", listener.local_addr().unwrap());
        let routes = routes(&origin);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let halt = stop.clone();
        let worker = thread::spawn(move || {
            while !halt.load(Ordering::Acquire) {
                let socket = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let Ok(mut stream) = acceptor.accept(socket) else {
                    continue;
                };
                let mut raw = Vec::new();
                let mut byte = [0];
                while raw.len() < 16384 && !raw.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    raw.push(byte[0]);
                }
                if !raw.ends_with(b"\r\n\r\n") {
                    continue;
                }
                let headers = String::from_utf8(raw).unwrap();
                let lower = headers.to_lowercase();
                assert!(
                    !lower.contains("authorization:")
                        && !lower.contains("cookie:")
                        && !lower.contains("proxy-authorization:")
                );
                let request = headers.lines().next().unwrap().to_owned();
                assert!(request.starts_with("GET "));
                let route = request.split_whitespace().nth(1).unwrap().to_owned();
                captured.lock().unwrap().push(route.clone());
                let empty = Reply::code(404);
                let reply = routes.get(&route).unwrap_or(&empty);
                let mut response = format!(
                    "HTTP/1.1 {} Owned fixture\r\nConnection: close\r\n",
                    reply.status
                );
                if !reply
                    .headers
                    .iter()
                    .any(|h| h.to_ascii_lowercase().starts_with("content-length:"))
                {
                    response.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
                }
                for h in &reply.headers {
                    response.push_str(h);
                    response.push_str("\r\n");
                }
                response.push_str("\r\n");
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&reply.body);
                let _ = stream.shutdown();
            }
        });
        Self {
            origin,
            pem,
            seen,
            stop,
            worker: Some(worker),
            _dir: dir,
            _exclusive: exclusive,
        }
    }
    fn network(&self) -> Network {
        Network::with_policy(
            Arc::new(SystemResolver),
            WebPolicy::with_private_origins(std::slice::from_ref(&self.origin)).unwrap(),
            std::slice::from_ref(&self.pem),
        )
        .unwrap()
    }
    fn paths(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn prm(origin: &str, resource: &str) -> Value {
    json!({"resource":format!("{origin}{resource}"),"authorization_servers":[format!("{origin}/tenant")],"scopes_supported":["files:read"]})
}
fn as_meta(origin: &str) -> Value {
    json!({"issuer":format!("{origin}/tenant"),"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"registration_endpoint":format!("{origin}/register"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"authorization_response_iss_parameter_supported":true})
}

#[test]
fn quoted_challenges_multiple_schemes_and_scope_priority_parse_literally() {
    let c=challenge::parse(&[r#"Basic realm="skip, me", Bearer realm="quoted\"realm", resource_metadata="https://owned.invalid/meta?x=a,b", scope="files:read files:write", error="insufficient_scope""#]).unwrap().unwrap();
    assert_eq!(
        c.resource_metadata.as_deref(),
        Some("https://owned.invalid/meta?x=a,b")
    );
    assert_eq!(
        c.scopes.unwrap(),
        BTreeSet::from(["files:read".into(), "files:write".into()])
    );
    assert!(c.insufficient_scope);
    assert!(
        challenge::parse(&["Negotiate TlRMTVNTUA==, Basic realm=owned"])
            .unwrap()
            .is_none()
    );
    assert!(challenge::parse(&["Bearer", "Bearer"]).unwrap().is_some());
}
#[test]
fn duplicate_ambiguous_malformed_and_oversized_challenges_refuse() {
    for bad in [
        r#"Bearer scope="read", Scope="write""#,
        r#"Bearer resource_metadata="https://a.invalid", Bearer resource_metadata="https://b.invalid""#,
        r#"Bearer scope="read\""#,
        r#"Bearer scope="read  write""#,
        r#"scope="read""#,
        "Bearer scope=read\r\nInjected: yes",
    ] {
        assert!(
            challenge::parse(&[bad]).is_err(),
            "accepted malformed challenge"
        );
    }
    assert!(challenge::parse(&[&"x".repeat(16385)]).is_err());
}
#[test]
fn path_prm_and_oauth_metadata_bind_exact_identities_without_other_calls() {
    let peer = Peer::new(|o| {
        BTreeMap::from([
            ("/mcp".into(), Reply::code(401)),
            (
                "/.well-known/oauth-protected-resource/mcp".into(),
                Reply::json(prm(o, "/mcp")),
            ),
            (
                "/.well-known/oauth-authorization-server/tenant".into(),
                Reply::json(as_meta(o)),
            ),
        ])
    });
    let net = peer.network();
    let (_source, mut cancel) = cancellation();
    rt().block_on(async {
        let resource = net
            .discover(&format!("{}/mcp", peer.origin), &mut cancel)
            .await
            .unwrap();
        assert_eq!(resource.view().resource, format!("{}/mcp", peer.origin));
        let auth = net
            .authorization(&resource, &format!("{}/tenant", peer.origin), &mut cancel)
            .await
            .unwrap();
        auth.check_code_flow().unwrap();
        assert!(auth.view().issuer_response_required);
        assert_eq!(auth.resource().endpoint, format!("{}/mcp", peer.origin));
        assert!(
            net.authorization(&resource, "https://foreign.invalid", &mut cancel)
                .await
                .is_err()
        );
    });
    assert_eq!(
        peer.paths(),
        [
            "/mcp",
            "/.well-known/oauth-protected-resource/mcp",
            "/.well-known/oauth-authorization-server/tenant"
        ]
    );
}
#[test]
fn challenge_metadata_and_challenged_scopes_override_well_known_and_metadata_scopes() {
    let peer = Peer::new(|o| {
        let mut response = Reply::code(401);
        response.headers.push(format!(
            "WWW-Authenticate: Bearer resource_metadata=\"{o}/custom\", scope=\"extra:write\""
        ));
        BTreeMap::from([
            ("/mcp".into(), response),
            ("/custom".into(), Reply::json(prm(o, "/mcp"))),
        ])
    });
    let (_source, mut cancel) = cancellation();
    let resource = rt()
        .block_on(
            peer.network()
                .discover(&format!("{}/mcp", peer.origin), &mut cancel),
        )
        .unwrap();
    assert_eq!(
        resource.view().scopes,
        BTreeSet::from(["extra:write".into()])
    );
    assert_eq!(peer.paths(), ["/mcp", "/custom"]);
}
#[test]
fn root_resource_fallback_keeps_exact_endpoint_and_oidc_paths_have_specified_order() {
    let peer = Peer::new(|o| {
        BTreeMap::from([
            ("/mcp".into(), Reply::code(405)),
            (
                "/.well-known/oauth-protected-resource".into(),
                Reply::json(prm(o, "")),
            ),
            (
                "/tenant/.well-known/openid-configuration".into(),
                Reply::json(as_meta(o)),
            ),
        ])
    });
    let (_source, mut cancel) = cancellation();
    rt().block_on(async {
        let net = peer.network();
        let resource = net
            .discover(&format!("{}/mcp", peer.origin), &mut cancel)
            .await
            .unwrap();
        assert_eq!(resource.view().resource, peer.origin);
        assert_eq!(resource.view().endpoint, format!("{}/mcp", peer.origin));
        net.authorization(&resource, &format!("{}/tenant", peer.origin), &mut cancel)
            .await
            .unwrap()
            .check_code_flow()
            .unwrap();
    });
    assert_eq!(
        peer.paths(),
        [
            "/mcp",
            "/.well-known/oauth-protected-resource/mcp",
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-authorization-server/tenant",
            "/.well-known/openid-configuration/tenant",
            "/tenant/.well-known/openid-configuration"
        ]
    );
}
#[test]
fn resource_and_issuer_substitution_never_fall_through_to_permissive_metadata() {
    for issuer in [false, true] {
        let peer = Peer::new(|o| {
            let mut metadata = as_meta(o);
            metadata["issuer"] = json!(format!("{o}/tenant/"));
            let mut resource = prm(o, "/mcp");
            if !issuer {
                resource["resource"] = json!(o);
            }
            BTreeMap::from([
                ("/mcp".into(), Reply::code(401)),
                (
                    "/.well-known/oauth-protected-resource/mcp".into(),
                    Reply::json(resource),
                ),
                (
                    "/.well-known/oauth-protected-resource".into(),
                    Reply::json(prm(o, "")),
                ),
                (
                    "/.well-known/oauth-authorization-server/tenant".into(),
                    Reply::json(metadata),
                ),
                (
                    "/.well-known/openid-configuration/tenant".into(),
                    Reply::json(as_meta(o)),
                ),
            ])
        });
        let (_source, mut cancel) = cancellation();
        rt().block_on(async {
            let net = peer.network();
            let resource = net
                .discover(&format!("{}/mcp", peer.origin), &mut cancel)
                .await;
            if issuer {
                assert!(
                    net.authorization(
                        &resource.unwrap(),
                        &format!("{}/tenant", peer.origin),
                        &mut cancel
                    )
                    .await
                    .is_err()
                );
            } else {
                assert!(resource.is_err());
            }
        });
        assert!(
            !peer
                .paths()
                .contains(&"/.well-known/openid-configuration/tenant".into())
        );
        assert!(
            !peer
                .paths()
                .contains(&"/.well-known/oauth-protected-resource".into())
        );
    }
}
#[test]
fn missing_or_plain_pkce_is_inspectable_but_cannot_start_code_flow() {
    for method in [None, Some("plain")] {
        let peer = Peer::new(|o| {
            let mut metadata = as_meta(o);
            metadata
                .as_object_mut()
                .unwrap()
                .remove("code_challenge_methods_supported");
            if let Some(m) = method {
                metadata["code_challenge_methods_supported"] = json!([m]);
            }
            BTreeMap::from([
                ("/mcp".into(), Reply::code(401)),
                (
                    "/.well-known/oauth-protected-resource/mcp".into(),
                    Reply::json(prm(o, "/mcp")),
                ),
                (
                    "/.well-known/oauth-authorization-server/tenant".into(),
                    Reply::json(metadata),
                ),
            ])
        });
        let (_source, mut cancel) = cancellation();
        rt().block_on(async {
            let net = peer.network();
            let resource = net
                .discover(&format!("{}/mcp", peer.origin), &mut cancel)
                .await
                .unwrap();
            let auth = net
                .authorization(&resource, &format!("{}/tenant", peer.origin), &mut cancel)
                .await
                .unwrap();
            assert!(!auth.view().pkce_s256);
            assert!(auth.check_code_flow().is_err());
        });
    }
}
#[test]
fn redirects_tls_failure_private_destinations_and_cancellation_send_no_credentials() {
    let peer = Peer::new(|o| {
        let mut r = Reply::code(302);
        r.headers.push(format!("Location: {o}/other"));
        BTreeMap::from([("/mcp".into(), r)])
    });
    let (_source, mut cancel) = cancellation();
    let runtime = rt();
    assert!(
        runtime
            .block_on(
                peer.network()
                    .discover(&format!("{}/mcp", peer.origin), &mut cancel)
            )
            .is_err()
    );
    assert_eq!(peer.paths(), ["/mcp"]);
    assert!(
        runtime
            .block_on(Network::default().discover(&format!("{}/mcp", peer.origin), &mut cancel))
            .is_err()
    );
    let untrusted = Network::with_policy(
        Arc::new(SystemResolver),
        WebPolicy::with_private_origins(std::slice::from_ref(&peer.origin)).unwrap(),
        &[],
    )
    .unwrap();
    assert!(
        runtime
            .block_on(untrusted.discover(&format!("{}/mcp", peer.origin), &mut cancel))
            .is_err()
    );
    let (source, mut cancelled) = cancellation();
    source.cancel();
    assert!(matches!(
        runtime.block_on(
            peer.network()
                .discover(&format!("{}/mcp", peer.origin), &mut cancelled)
        ),
        Err(DomainError::Cancelled)
    ));
    assert_eq!(peer.paths(), ["/mcp"]);
}
struct PrivateDns;
impl WebResolver for PrivateDns {
    fn resolve<'a>(&'a self, _: &'a str, _: u16) -> PortFuture<'a, Vec<IpAddr>> {
        Box::pin(async {
            Ok(vec![
                "8.8.8.8".parse().unwrap(),
                "127.0.0.1".parse().unwrap(),
            ])
        })
    }
}
#[test]
fn mixed_dns_answers_and_credential_urls_refuse_before_network() {
    let network = Network::with_policy(Arc::new(PrivateDns), WebPolicy::default(), &[]).unwrap();
    let (_source, mut cancel) = cancellation();
    let runtime = rt();
    for target in [
        "https://owned.invalid/mcp",
        "http://owned.invalid/mcp",
        "https://user:password@owned.invalid/mcp",
        "https://owned.invalid/mcp?access_token=bad",
        "https://owned.invalid/mcp#fragment",
    ] {
        assert!(
            runtime
                .block_on(network.discover(target, &mut cancel))
                .is_err()
        );
    }
}
#[test]
fn metadata_quotas_duplicate_keys_types_and_truncation_fail_closed() {
    for mode in 0..5 {
        let peer = Peer::new(|o| {
            let mut r = Reply::json(prm(o, "/mcp"));
            match mode {
                0 => r.headers.push("Content-Length: 262145".into()),
                1 => r.body = b"{\"resource\":\"one\",\"resource\":\"two\"}".to_vec(),
                2 => r.headers = vec!["Content-Type: text/html".into()],
                3 => r.body = b"{".to_vec(),
                _ => r.headers.push("Content-Encoding: gzip".into()),
            }
            BTreeMap::from([
                ("/mcp".into(), Reply::code(401)),
                ("/.well-known/oauth-protected-resource/mcp".into(), r),
            ])
        });
        let (_source, mut cancel) = cancellation();
        assert!(
            rt().block_on(
                peer.network()
                    .discover(&format!("{}/mcp", peer.origin), &mut cancel)
            )
            .is_err()
        );
        assert_eq!(peer.paths().len(), 2);
    }
}

#[cfg(target_os = "linux")]
mod callbacks {
    use super::*;
    use ag1_core::SecretBytes;
    use ag1_platform::{
        mcp::{
            auth::{Binding, Target},
            oauth::login::{Browser, ClientRegistration, Login},
        },
        provider::http::CancelToken,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Default)]
    struct OwnedBrowser(Mutex<Option<String>>);
    impl Browser for OwnedBrowser {
        fn open_authorization(&self, url: &SecretBytes) -> Result<(), DomainError> {
            *self.0.lock().unwrap() = Some(std::str::from_utf8(url.expose()).unwrap().into());
            Ok(())
        }
    }
    fn peer() -> Peer {
        Peer::new(|o| {
            BTreeMap::from([
                ("/mcp".into(), Reply::code(401)),
                (
                    "/.well-known/oauth-protected-resource/mcp".into(),
                    Reply::json(prm(o, "/mcp")),
                ),
                (
                    "/.well-known/oauth-authorization-server/tenant".into(),
                    Reply::json(as_meta(o)),
                ),
            ])
        })
    }
    async fn login(peer: &Peer, cancel: &mut CancelToken) -> Login {
        let net = peer.network();
        let resource = net
            .discover(&format!("{}/mcp", peer.origin), cancel)
            .await
            .unwrap();
        let issuer = format!("{}/tenant", peer.origin);
        let auth = net.authorization(&resource, &issuer, cancel).await.unwrap();
        let binding = Binding {
            server_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
            profile_id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into(),
            configuration_revision: 1,
            configuration_sha256: "a".repeat(64),
            target: Target::OAuth {
                resource: resource.view().resource.clone(),
                issuer: issuer.clone(),
            },
        };
        net.begin_login(
            auth,
            ClientRegistration::pre_registered(&issuer, "owned-client").unwrap(),
            binding,
            cancel,
        )
        .await
        .unwrap()
    }
    fn parameters(browser: &OwnedBrowser) -> BTreeMap<String, String> {
        reqwest::Url::parse(browser.0.lock().unwrap().as_ref().unwrap())
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }
    fn query(pairs: &[(&str, &str)]) -> String {
        let mut url = reqwest::Url::parse("http://127.0.0.1/").unwrap();
        url.query_pairs_mut().extend_pairs(pairs.iter().copied());
        url.query().unwrap().into()
    }
    async fn callback(redirect: &str, path: Option<&str>, query: &str, extra: &str) -> String {
        let uri = reqwest::Url::parse(redirect).unwrap();
        let mut socket =
            tokio::net::TcpStream::connect((uri.host_str().unwrap(), uri.port().unwrap()))
                .await
                .unwrap();
        let host = &redirect[7..redirect[7..].find('/').unwrap() + 7];
        let request = format!(
            "GET {}?{query} HTTP/1.1\r\nHost: {host}\r\n{extra}\r\n",
            path.unwrap_or(uri.path())
        );
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        tokio::time::timeout(Duration::from_secs(3), socket.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(!response.contains("owned-code"));
        assert!(!response.contains("owned-error-description"));
        response
    }
    async fn closed(redirect: &str) {
        let uri = reqwest::Url::parse(redirect).unwrap();
        assert!(
            tokio::net::TcpStream::connect((uri.host_str().unwrap(), uri.port().unwrap()))
                .await
                .is_err()
        );
    }

    #[test]
    fn loopback_login_rejects_unbound_requests_then_consumes_exact_callback_once() {
        let peer = peer();
        rt().block_on(async {
            let (_source, mut cancel) = cancellation();
            let mut pending = login(&peer, &mut cancel).await;
            let redirect = pending.redirect_uri().to_owned();
            let browser = OwnedBrowser::default();
            pending.open(&browser, &cancel).unwrap();
            assert!(pending.open(&browser, &cancel).is_err());
            let params = parameters(&browser);
            assert_eq!(params["client_id"], "owned-client");
            assert_eq!(params["response_type"], "code");
            assert_eq!(params["resource"], format!("{}/mcp", peer.origin));
            assert_eq!(params["scope"], "files:read");
            assert_eq!(params["redirect_uri"], redirect);
            assert_eq!(params["code_challenge_method"], "S256");
            assert_eq!(params["code_challenge"].len(), 43);
            assert_eq!(params["state"].len(), 43);
            assert!(!params.contains_key("code_verifier"));
            let issuer = format!("{}/tenant", peer.origin);
            let good = query(&[
                ("state", &params["state"]),
                ("iss", &issuer),
                ("code", "owned-code"),
            ]);
            let sender = async {
                let bad = query(&[("state", "wrong"), ("iss", &issuer), ("code", "owned-code")]);
                assert!(
                    callback(&redirect, None, &bad, "")
                        .await
                        .starts_with("HTTP/1.1 400")
                );
                assert!(
                    callback(&redirect, Some("/wrong"), &good, "")
                        .await
                        .starts_with("HTTP/1.1 400")
                );
                assert!(
                    callback(&redirect, None, &good, "Host: evil.invalid\r\n")
                        .await
                        .starts_with("HTTP/1.1 400")
                );
                assert!(
                    callback(&redirect, None, &format!("{good}&state=duplicate"), "")
                        .await
                        .starts_with("HTTP/1.1 400")
                );
                assert!(
                    callback(&redirect, None, &format!("{good}&x=%ZZ"), "")
                        .await
                        .starts_with("HTTP/1.1 400")
                );
                let reply = callback(&redirect, None, &good, "").await;
                assert!(reply.starts_with("HTTP/1.1 200"));
                assert!(reply.contains("Cache-Control: no-store"));
                assert!(reply.contains("Referrer-Policy: no-referrer"));
                assert!(!reply.contains(&params["state"]));
            };
            let (result, ()) = tokio::join!(pending.wait(&mut cancel), sender);
            let code = result.unwrap();
            assert_eq!(code.binding().configuration_revision, 1);
            closed(&redirect).await;
        });
        assert!(
            peer.paths()
                .iter()
                .all(|p| !p.starts_with("/token") && !p.starts_with("/authorize"))
        );
    }

    #[test]
    fn callback_issuer_is_exact_on_success_and_error_and_errors_never_reflect_input() {
        let peer = peer();
        rt().block_on(async {
            for mode in 0..6 {
                let (_source, mut cancel) = cancellation();
                let mut pending = login(&peer, &mut cancel).await;
                let redirect = pending.redirect_uri().to_owned();
                let browser = OwnedBrowser::default();
                pending.open(&browser, &cancel).unwrap();
                let params = parameters(&browser);
                let issuer = format!("{}/tenant", peer.origin);
                let wrong = format!("{issuer}/");
                let mut pairs = vec![("state", params["state"].as_str())];
                if mode != 0 {
                    pairs.push((
                        "iss",
                        if matches!(mode, 1 | 3) {
                            &wrong
                        } else {
                            &issuer
                        },
                    ));
                }
                if mode < 2 || mode == 4 {
                    pairs.push(("code", "owned-code"));
                }
                if mode >= 2 {
                    pairs.extend([
                        ("error", "access_denied"),
                        ("error_description", "owned-error-description"),
                    ]);
                }
                let q = query(&pairs);
                let (result, response) =
                    tokio::join!(pending.wait(&mut cancel), callback(&redirect, None, &q, ""));
                assert!(result.is_err());
                assert!(response.starts_with("HTTP/1.1 400"));
                let error = format!("{:?}", result.err().unwrap());
                assert!(
                    !error.contains("owned-code") && !error.contains("owned-error-description")
                );
                if mode == 0 {
                    assert!(error.contains("issuer missing"));
                }
                if matches!(mode, 1 | 3) {
                    assert!(error.contains("issuer mismatch"));
                }
                closed(&redirect).await;
            }
        });
    }

    #[test]
    fn cancellation_abandonment_unopened_login_and_attempt_budget_close_listener() {
        let peer = peer();
        rt().block_on(async {
            let (source, mut cancel) = cancellation();
            let mut pending = login(&peer, &mut cancel).await;
            let redirect = pending.redirect_uri().to_owned();
            pending.open(&OwnedBrowser::default(), &cancel).unwrap();
            let canceller = async {
                tokio::task::yield_now().await;
                source.cancel();
            };
            let (result, ()) = tokio::join!(pending.wait(&mut cancel), canceller);
            assert!(matches!(result, Err(DomainError::Cancelled)));
            closed(&redirect).await;
            let (_source, mut cancel) = cancellation();
            let pending = login(&peer, &mut cancel).await;
            let redirect = pending.redirect_uri().to_owned();
            assert!(matches!(
                pending.wait(&mut cancel).await,
                Err(DomainError::PermissionDenied)
            ));
            closed(&redirect).await;
            let pending = login(&peer, &mut cancel).await;
            let redirect = pending.redirect_uri().to_owned();
            drop(pending);
            closed(&redirect).await;
            let mut pending = login(&peer, &mut cancel).await;
            let redirect = pending.redirect_uri().to_owned();
            pending.open(&OwnedBrowser::default(), &cancel).unwrap();
            let sender = async {
                for _ in 0..32 {
                    assert!(
                        callback(&redirect, None, "state=wrong", "")
                            .await
                            .starts_with("HTTP/1.1 400")
                    );
                }
            };
            let (result, ()) = tokio::join!(pending.wait(&mut cancel), sender);
            assert!(matches!(result, Err(DomainError::LimitExceeded(_))));
            closed(&redirect).await;
        });
    }
}
