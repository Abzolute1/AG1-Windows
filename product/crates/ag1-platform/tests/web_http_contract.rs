use ag1_core::{DomainError, PortFuture, SecretBytes};
use ag1_platform::{provider::http::cancellation, web_http::*};
use std::{
    io::{Read, Write},
    net::{IpAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use tokio::time::Instant;

// These independent cases share the production process-wide fetch semaphore.
// Serialize case setup; concurrency exercised inside a case remains unchanged.
static WEB_CASE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = vec![];
    let mut buffer = [0; 1024];
    while !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() < 32768);
    }
    String::from_utf8(bytes).unwrap()
}
fn server(
    count: usize,
    response: impl Fn(usize, &str) -> Vec<u8> + Send + 'static,
) -> (u16, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut requests = vec![];
        while requests.len() < count {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    let input = request(&mut socket);
                    let output = response(requests.len(), &input);
                    let _ = socket.write_all(&output);
                    requests.push(input);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "fixture server missing request"
                    );
                    thread::sleep(Duration::from_millis(2));
                }
                Err(e) => panic!("{e}"),
            }
        }
        requests
    });
    (port, task)
}
fn ok(body: &[u8]) -> Vec<u8> {
    let mut bytes=format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).into_bytes();
    bytes.extend(body);
    bytes
}
struct Resolver {
    calls: Arc<AtomicUsize>,
    hosts: Arc<Mutex<Vec<String>>>,
    answers: Vec<Vec<IpAddr>>,
}
impl WebResolver for Resolver {
    fn resolve<'a>(&'a self, host: &'a str, _port: u16) -> PortFuture<'a, Vec<IpAddr>> {
        self.hosts.lock().unwrap().push(host.into());
        let i = self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(self.answers[i.min(self.answers.len() - 1)].clone()) })
    }
}
#[test]
fn addresses_urls_and_credentials_are_checked_before_network_access() {
    let _case = WEB_CASE.blocking_lock();
    let denied = [
        "0.1.2.3",
        "10.1.1.1",
        "100.64.0.1",
        "100.127.255.255",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "172.31.255.255",
        "192.0.0.1",
        "192.0.2.4",
        "192.168.1.2",
        "198.18.0.1",
        "198.19.255.255",
        "198.51.100.1",
        "203.0.113.8",
        "224.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::ffff:127.0.0.1",
        "::ffff:c0a8:102",
        "fc00::1",
        "fdff::1",
        "fe80::1",
        "febf::1",
        "ff02::1",
        "64:ff9b::7f00:1",
        "100::1",
        "2001:db8::1",
        "2001::1",
        "2002:7f00:1::1",
    ];
    for value in denied {
        assert!(!public_address(value.parse().unwrap()), "{value}");
    }
    for value in [
        "1.1.1.1",
        "8.8.8.8",
        "100.63.255.255",
        "100.128.0.0",
        "172.15.255.255",
        "172.32.0.0",
        "192.0.3.1",
        "198.17.255.255",
        "198.20.0.1",
        "2001:4860:4860::8888",
        "2606:4700:4700::1111",
        "::ffff:8.8.8.8",
    ] {
        assert!(public_address(value.parse().unwrap()), "{value}");
    }
    let policy = WebPolicy::default();
    let public = ["8.8.8.8".parse().unwrap()];
    // Exact selected legacy spellings, checked even with an otherwise public DNS answer.
    for url in [
        "http://[::ffff:127.0.0.1]/",
        "http://[::ffff:7f00:1]/",
        "http://[::ffff:10.1.2.3]/",
        "http://[::ffff:ac10:1]/",
        "http://[::ffff:192.168.0.1]/",
        "http://127.0.0.1/",
        "http://[::1]/",
        "http://[0:0:0:0:0:0:0:1]/",
        "http://[0:0:0:0:0:ffff:127.0.0.1]/",
    ] {
        assert_eq!(
            policy.validate(url, &public),
            Err(DomainError::PermissionDenied),
            "{url}"
        );
    }
    for url in [
        "file:///tmp/foo",
        "ftp://example.com/a",
        "http://secret@example.com/",
        "https://u:p@example.com/",
        "https://localhost/",
        "https://sub.localhost./",
    ] {
        assert!(policy.validate(url, &public).is_err(), "{url}");
    }
    assert_eq!(
        policy.validate(
            "https://example.com/",
            &[public[0], "127.0.0.1".parse().unwrap()]
        ),
        Err(DomainError::PermissionDenied)
    );
    assert!(policy.validate("https://example.com/", &[]).is_err());
    assert!(
        policy
            .validate("https://example.com/", &[public[0]; 33])
            .is_err()
    );
    let fixture = WebPolicy::with_private_origins(&["http://fixture.invalid:1234".into()]).unwrap();
    assert!(
        fixture
            .validate(
                "http://fixture.invalid:1234/a",
                &["127.0.0.1".parse().unwrap()]
            )
            .is_ok()
    );
    assert_eq!(
        fixture.validate(
            "http://fixture.invalid:1235/a",
            &["127.0.0.1".parse().unwrap()]
        ),
        Err(DomainError::PermissionDenied)
    );
    let secret = SecretBytes::new(b"Bearer fixture-only".to_vec());
    let credential =
        WebCredential::new(&[".Example.com".into()], "authorization", &secret).unwrap();
    for url in ["https://example.com/a", "https://sub.example.com/a"] {
        assert!(credential.permits(url));
    }
    for url in [
        "http://example.com/",
        "https://badexample.com/",
        "https://example.com.evil.invalid/",
        "https://u:p@example.com/",
    ] {
        assert!(!credential.permits(url));
    }
    for header in [
        "Host",
        "Content-Length",
        "Transfer-Encoding",
        "Connection",
        "Proxy-Authorization",
        "Cookie",
    ] {
        assert!(WebCredential::new(&["example.com".into()], header, &secret).is_err());
    }
}
#[tokio::test]
async fn pinned_dns_relative_redirect_and_provenance_use_only_checked_destinations() {
    let _case = WEB_CASE.lock().await;
    let (port, task) = server(2, |index, input| {
        if index == 0 {
            assert!(input.starts_with("GET /start "));
            b"HTTP/1.1 302 Found\r\nLocation: /final#fragment\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            assert!(input.starts_with("GET /final "));
            ok("\u{feff}hello 🦙".as_bytes())
        }
    });
    let origin = format!("http://fixture.invalid:{port}");
    let calls = Arc::new(AtomicUsize::new(0));
    let hosts = Arc::new(Mutex::new(vec![]));
    let resolver = Resolver {
        calls: calls.clone(),
        hosts: hosts.clone(),
        answers: vec![vec!["127.0.0.1".parse().unwrap()]],
    };
    let http = WebHttp::with_resolver(
        WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap(),
        resolver,
    );
    let (_source, mut cancel) = cancellation();
    let response = http
        .fetch(
            &format!("{origin}/start"),
            None,
            Instant::now() + Duration::from_secs(3),
            &mut cancel,
        )
        .await
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one DNS resolution per hop, no transport re-resolution"
    );
    assert_eq!(
        *hosts.lock().unwrap(),
        vec!["fixture.invalid", "fixture.invalid"]
    );
    assert_eq!(response.requested_url, format!("{origin}/start"));
    assert_eq!(response.final_url, format!("{origin}/final"));
    assert_eq!(response.redirect_chain, vec![format!("{origin}/start")]);
    assert_eq!(response.status, 200);
    assert_eq!(response.bytes, "\u{feff}hello 🦙".as_bytes());
    for input in task.join().unwrap() {
        assert!(
            input
                .to_lowercase()
                .contains(&format!("host: fixture.invalid:{port}"))
        );
        assert!(!input.to_lowercase().contains("authorization:"));
    }
}
#[tokio::test]
async fn redirect_cannot_expand_private_origin_exception_or_embed_credentials() {
    let _case = WEB_CASE.lock().await;
    for destination in [
        "http://127.0.0.1:1/private",
        "http://secret@fixture.invalid/",
        "file:///etc/passwd",
    ] {
        let destination = destination.to_string();
        let (port, task) = server(1, move |_, _| {
            format!("HTTP/1.1 302 Found\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
        });
        let origin = format!("http://127.0.0.1:{port}");
        let http =
            WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap());
        let (_source, mut cancel) = cancellation();
        assert_eq!(
            http.fetch(
                &origin,
                None,
                Instant::now() + Duration::from_secs(2),
                &mut cancel
            )
            .await
            .unwrap_err(),
            DomainError::PermissionDenied
        );
        assert_eq!(task.join().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn body_length_stream_limit_partial_eof_redirect_loop_and_http_errors_settle() {
    let _case = WEB_CASE.lock().await;
    let cases = [
        (
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_FETCH_BYTES + 1
            )
            .into_bytes(),
            "length",
        ),
        (
            {
                let mut bytes =
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                        .to_vec();
                write!(&mut bytes, "{:x}\r\n", MAX_FETCH_BYTES + 1).unwrap();
                bytes.extend(vec![b'x'; MAX_FETCH_BYTES + 1]);
                bytes.extend(b"\r\n0\r\n\r\n");
                bytes
            },
            "body",
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort".to_vec(),
            "partial",
        ),
        (
            b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            "location",
        ),
        (
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            "status",
        ),
    ];
    for (bytes, kind) in cases {
        let (port, task) = server(1, move |_, _| bytes.clone());
        let origin = format!("http://127.0.0.1:{port}");
        let http =
            WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap());
        let (_source, mut cancel) = cancellation();
        let error = http
            .fetch(
                &origin,
                None,
                Instant::now() + Duration::from_secs(3),
                &mut cancel,
            )
            .await
            .unwrap_err();
        match kind {
            "length" | "body" => {
                assert!(matches!(error, DomainError::LimitExceeded(_)), "{error:?}")
            }
            "location" => assert!(matches!(error, DomainError::InvalidInput(_))),
            _ => assert!(matches!(error, DomainError::Unavailable(_))),
        }
        task.join().unwrap();
    }
    let (port, task) = server(6, |_, _| {
        b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    });
    let origin = format!("http://127.0.0.1:{port}");
    let http =
        WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap());
    let (_source, mut cancel) = cancellation();
    assert!(matches!(
        http.fetch(
            &origin,
            None,
            Instant::now() + Duration::from_secs(3),
            &mut cancel
        )
        .await,
        Err(DomainError::LimitExceeded(_))
    ));
    assert_eq!(task.join().unwrap().len(), 6);
}
#[tokio::test]
async fn cancellation_and_deadline_cover_dns_headers_and_body() {
    let _case = WEB_CASE.lock().await;
    struct SlowDns;
    impl WebResolver for SlowDns {
        fn resolve<'a>(&'a self, _: &'a str, _: u16) -> PortFuture<'a, Vec<IpAddr>> {
            Box::pin(std::future::pending())
        }
    }
    let http = WebHttp::with_resolver(WebPolicy::default(), SlowDns);
    let (_source, mut cancel) = cancellation();
    assert_eq!(
        http.fetch(
            "https://fixture.invalid/",
            None,
            Instant::now() + Duration::from_millis(20),
            &mut cancel
        )
        .await
        .unwrap_err(),
        DomainError::TimedOut
    );
    let (source, mut cancel) = cancellation();
    source.cancel();
    assert_eq!(
        http.fetch(
            "https://fixture.invalid/",
            None,
            Instant::now() + Duration::from_secs(1),
            &mut cancel
        )
        .await
        .unwrap_err(),
        DomainError::Cancelled
    );
    for cancel_request in [false, true] {
        let (source, mut cancel) = cancellation();
        let source = Arc::new(source);
        let (port, task) = server(1, move |_, _| {
            if cancel_request {
                source.cancel();
            }
            thread::sleep(Duration::from_millis(120));
            ok(b"late")
        });
        let origin = format!("http://127.0.0.1:{port}");
        let http =
            WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap());
        assert_eq!(
            http.fetch(
                &origin,
                None,
                Instant::now() + Duration::from_millis(60),
                &mut cancel
            )
            .await
            .unwrap_err(),
            if cancel_request {
                DomainError::Cancelled
            } else {
                DomainError::TimedOut
            }
        );
        task.join().unwrap();
    }
    // Headers arrive, but EOF does not; neither partial bytes nor completion escape.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        request(&mut socket);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\npartial")
            .unwrap();
        thread::sleep(Duration::from_millis(120));
    });
    let http =
        WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap());
    let (_source, mut cancel) = cancellation();
    assert_eq!(
        http.fetch(
            &origin,
            None,
            Instant::now() + Duration::from_millis(60),
            &mut cancel
        )
        .await
        .unwrap_err(),
        DomainError::TimedOut
    );
    task.join().unwrap();
}

#[test]
fn reference_url_policy_cases_preserve_rules_and_name_security_corrections() {
    let _case = WEB_CASE.blocking_lock();
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/webfetch-oracle.json"
    ))
    .unwrap();
    let cases = oracle["policy"].as_array().unwrap();
    assert_eq!(cases.len(), 36);
    let mut corrections = vec![];
    for (index, row) in cases.iter().enumerate() {
        let value = row["url"].as_str().unwrap();
        let addresses = if let Some(values) = row["addresses"].as_array() {
            values
                .iter()
                .map(|v| v.as_str().unwrap().parse().unwrap())
                .collect::<Vec<IpAddr>>()
        } else {
            vec![
                reqwest::Url::parse(value)
                    .ok()
                    .and_then(|u| {
                        u.host_str()
                            .and_then(|h| h.trim_matches(['[', ']']).parse().ok())
                    })
                    .unwrap_or("1.1.1.1".parse().unwrap()),
            ]
        };
        let legacy = row["result"]["ok"].as_bool().unwrap();
        let corrected = match index {
            22 => Some("full IPv6 link-local /10 block"),
            26 => Some("IPv6 6to4 private-address tunneling block"),
            27 => Some("IPv6 Teredo tunneling block"),
            30 => Some("embedded credential rejection"),
            31 => Some("trailing-dot localhost rejection"),
            33 => Some("empty DNS answer rejection"),
            35 => Some("DNS IPv6 link-local /10 block"),
            _ => None,
        };
        let actual = WebPolicy::default().validate(value, &addresses).is_ok();
        if let Some(reason) = corrected {
            assert!(legacy, "reference correction {index} no longer selected");
            assert!(!actual, "{reason}");
            corrections.push(reason);
        } else {
            assert_eq!(actual, legacy, "URL policy {index}: {value}");
        }
    }
    assert_eq!(corrections.len(), 7);
}

#[test]
fn approved_fetch_persists_full_untrusted_evidence_and_replays_without_network() {
    let _case = WEB_CASE.blocking_lock();
    use ag1_core::{
        Clock, SessionId,
        context::{ContextArtifactPort, features::Features},
        tools::{
            approval::{Authorizer, ExecutionFence, Preflight, PreflightContext},
            executor::{self, Execution, ExecutionArtifacts, NoHooks},
            policy::Policy,
            registry::{Registry, Scope},
            research,
            result::ResultBudget,
        },
    };
    use ag1_platform::web_tools::{WebExecution, WebService};
    use ag1_storage::{Store, context::ContextStore};
    use serde_json::json;
    struct Fixed(u64);
    impl Clock for Fixed {
        fn now_ms(&self) -> u64 {
            self.0
        }
    }
    let (port, task) = server(4, |index, _| {
        if index % 2 == 0 {
            b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            let mut response=ok(b"First useful observation contains the parameter and its behavior.\nSecond useful observation explains when to use the example.");
            response.splice(9..12, b"201".iter().copied());
            response
        }
    });
    let origin = format!("http://127.0.0.1:{port}");
    let url = format!("{origin}/start");
    let scope = Scope {
        workspace: "fixture-workspace".into(),
        session: "fixture".into(),
        epoch: 1,
        revision: 1,
    };
    let service = WebService::new(
        WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap()),
        scope.clone(),
        "web-fixture-incarnation",
        Default::default(),
    )
    .unwrap();
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(&["webFetch".into()], &["webFetch".into()].into())
        .unwrap();
    let policy: Policy =
        serde_json::from_value(json!({"mode":"interactive","cwd":"/fixture"})).unwrap();
    let authorize = |call, now| {
        let mut auth = Authorizer::new("web-approval-fixture").unwrap();
        let result = auth
            .preflight(
                call,
                PreflightContext {
                    scope: &scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: service.target_revision(),
                    now,
                    ttl: 1000,
                    untrusted_extensions: false,
                },
            )
            .unwrap();
        match result {
            Preflight::Ready(t) => t,
            Preflight::Approval(a) => auth
                .answer(
                    &a.binding,
                    true,
                    &registry,
                    ExecutionFence {
                        scope: &scope,
                        policy_revision: 1,
                        target_revision: service.target_revision(),
                        now,
                    },
                )
                .unwrap()
                .unwrap(),
            _ => panic!("unexpected rejection"),
        }
    };
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).unwrap();
    let timestamp = "2026-09-05T00:00:00.000Z";
    store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":data.path(),"timestamp":timestamp}).to_string().as_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[4; 32])
        .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut events = vec![];
    for (id, now, cache) in [
        ("first", 12, false),
        ("cached", 13, true),
        ("expired", 900013, false),
    ] {
        let call = registry
            .prepare(id, id, "webFetch", &json!({"url":url}), true)
            .unwrap();
        let mut artifacts = ContextStore::new(
            &store,
            SessionId::new("fixture").unwrap(),
            "fixture",
            id,
            timestamp,
            Features::default(),
        )
        .unwrap();
        let (_source, mut cancel) = cancellation();
        let result = executor::execute_with_artifacts(
            &store,
            &mut artifacts,
            authorize(call.clone(), now),
            Execution {
                registry: &registry,
                fence: ExecutionFence {
                    scope: &scope,
                    policy_revision: 1,
                    target_revision: service.target_revision(),
                    now,
                },
                budget: ResultBudget::default(),
                clock: &Fixed(now),
            },
            &mut NoHooks,
            |ticket, artifacts| {
                runtime.block_on(service.execute(
                    ticket,
                    WebExecution {
                        registry: &registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || now,
                        deadline: Instant::now() + Duration::from_secs(3),
                        cancel: &mut cancel,
                        profile: research::profile(90000., 0.),
                    },
                    artifacts,
                ))
            },
            |e| events.push(e),
        )
        .unwrap();
        assert_eq!(result.result["details"]["cacheHit"], cache);
        assert_eq!(
            result.result["details"]["finalUrl"],
            format!("{origin}/final")
        );
        assert_eq!(result.result["details"]["statusCode"], 201);
        let evidence = &result.result["details"]["networkEvidence"];
        assert_eq!(evidence["trust"], "untrusted");
        assert!(result.result["details"].get("inspectionEvidence").is_none());
        let metadata = artifacts
            .get(evidence["artifactId"].as_str().unwrap())
            .unwrap();
        let raw = artifacts.read_json(&metadata).unwrap();
        assert_eq!(raw["finalUrl"], format!("{origin}/final"));
        assert_eq!(raw["statusCode"], 201);
        assert_eq!(raw["trust"], "untrusted");
        assert!(raw["bodyBase64"].as_str().unwrap().len() > 100);
        let before = events.len();
        let replay = executor::execute_with_artifacts(
            &store,
            &mut artifacts,
            authorize(call, now),
            Execution {
                registry: &registry,
                fence: ExecutionFence {
                    scope: &scope,
                    policy_revision: 1,
                    target_revision: service.target_revision(),
                    now,
                },
                budget: ResultBudget::default(),
                clock: &Fixed(now),
            },
            &mut NoHooks,
            |_, _| panic!("completed fetch was re-executed"),
            |e| events.push(e),
        )
        .unwrap();
        assert_eq!(replay.result, result.result);
        assert_eq!(events.len(), before);
    }
    assert_eq!(events.len(), 6);
    assert_eq!(task.join().unwrap().len(), 4);
    assert_eq!(service.cache_usage().unwrap().0, 1);
    let mut foreign = scope.clone();
    foreign.epoch += 1;
    assert_eq!(
        service.outcome(&foreign, &url, 900013),
        Err(DomainError::PermissionDenied)
    );
    assert!(service.outcome(&scope, &url, 1800014).unwrap().is_none());
    drop(store);
    let reopened = Store::open(data.path()).unwrap();
    let artifacts = ContextStore::new(
        &reopened,
        SessionId::new("fixture").unwrap(),
        "fixture",
        "reopened",
        timestamp,
        Features::default(),
    )
    .unwrap();
    let call = registry
        .prepare("first", "first", "webFetch", &json!({"url":url}), true)
        .unwrap();
    assert!(
        executor::replay(&reopened, &artifacts, &call, &scope)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn compressed_entities_are_bounded_after_decoding_and_match_reference_projections() {
    let _case = WEB_CASE.lock().await;
    use ag1_core::tools::research;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use sha2::{Digest, Sha256};
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/web-compression-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 7);
    for (index, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let compressed = STANDARD
            .decode(input["compressedBase64"].as_str().unwrap())
            .unwrap();
        let codec = input["codec"].as_str().unwrap().to_owned();
        let (port, task) = server(1, move |_, _| {
            let mut bytes=format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Encoding: {codec}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",compressed.len()).into_bytes();
            bytes.extend_from_slice(&compressed);
            bytes
        });
        let origin = format!("http://127.0.0.1:{port}");
        let http =
            WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap());
        let (_source, mut cancel) = cancellation();
        let outcome = http
            .fetch(
                &origin,
                None,
                Instant::now() + Duration::from_secs(3),
                &mut cancel,
            )
            .await;
        if input["size"].as_u64().unwrap() > MAX_FETCH_BYTES as u64 {
            assert!(matches!(outcome, Err(DomainError::LimitExceeded(_))));
            assert!(row["output"]["details"]["error"].as_str().is_some());
        } else {
            let response = outcome.unwrap();
            assert_eq!(
                response.bytes.len(),
                input["size"].as_u64().unwrap() as usize
            );
            assert_eq!(
                format!("{:x}", Sha256::digest(&response.bytes)),
                input["decodedSha256"]
            );
            let projected = research::fetch_projection(
                research::FetchedDocument {
                    url: "http://fixture.invalid/",
                    final_url: "http://fixture.invalid/",
                    status_code: response.status,
                    content_type: Some(&response.content_type),
                    cache_hit: false,
                    bytes: &response.bytes,
                },
                research::profile(0., 0.),
                None,
            )
            .unwrap();
            assert_eq!(
                projected.result, row["output"],
                "decoded projection {index}"
            );
        }
        task.join().unwrap();
    }
}

fn tls_server(
    count: usize,
    response: impl Fn(usize, &str, u16) -> Vec<u8> + Send + 'static,
) -> (u16, Vec<u8>, thread::JoinHandle<Vec<String>>) {
    let dir = tempfile::tempdir().unwrap();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    let output = std::process::Command::new("/usr/bin/openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=fixture.invalid",
            "-addext",
            "subjectAltName=DNS:fixture.invalid,DNS:outside.invalid,DNS:brave.fixture.invalid",
            "-keyout",
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated TLS fixture generation failed"
    );
    let pem = std::fs::read(cert).unwrap();
    let identity = native_tls::Identity::from_pkcs8(&pem, &std::fs::read(key).unwrap()).unwrap();
    let acceptor = native_tls::TlsAcceptor::new(identity).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut requests = vec![];
        let mut accepts = 0;
        while accepts < count {
            match listener.accept() {
                Ok((socket, _)) => {
                    accepts += 1;
                    socket
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    socket
                        .set_write_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    match acceptor.accept(socket) {
                        Ok(mut stream) => {
                            let mut bytes = vec![];
                            let mut buffer = [0; 1024];
                            while !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
                                let count = stream.read(&mut buffer).unwrap();
                                assert!(count > 0);
                                bytes.extend_from_slice(&buffer[..count]);
                                assert!(bytes.len() < 32768);
                            }
                            let input = String::from_utf8(bytes).unwrap();
                            let output = response(requests.len(), &input, port);
                            let _ = stream.write_all(&output);
                            requests.push(input);
                            let _ = stream.shutdown();
                        }
                        Err(_) => requests.push("TLS rejected before request".into()),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "missing TLS fixture connection"
                    );
                    thread::sleep(Duration::from_millis(2));
                }
                Err(e) => panic!("{e}"),
            }
        }
        requests
    });
    (port, pem, task)
}
fn fixed_resolver() -> Resolver {
    Resolver {
        calls: Arc::new(AtomicUsize::new(0)),
        hosts: Arc::new(Mutex::new(vec![])),
        answers: vec![vec!["127.0.0.1".parse().unwrap()]],
    }
}
#[tokio::test]
async fn real_tls_checks_hostname_and_chain_and_strips_credentials_on_cross_host_redirect() {
    let _case = WEB_CASE.lock().await;
    let (port, certificate, task) = tls_server(2, |index, input, port| {
        if index == 0 {
            assert!(
                input
                    .to_lowercase()
                    .contains("authorization: bearer fixture-only")
            );
            format!("HTTP/1.1 302 Found\r\nLocation: https://outside.invalid:{port}/final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
        } else {
            assert!(!input.to_lowercase().contains("authorization:"));
            ok(b"credential did not follow redirect")
        }
    });
    let origin = format!("https://fixture.invalid:{port}");
    let outside = format!("https://outside.invalid:{port}");
    let http = WebHttp::with_resolver(
        WebPolicy::with_private_origins(&[origin.clone(), outside.clone()]).unwrap(),
        fixed_resolver(),
    )
    .with_certificates(&[certificate])
    .unwrap();
    let credential = WebCredential::new(
        &["fixture.invalid".into()],
        "authorization",
        &SecretBytes::new(b"Bearer fixture-only".to_vec()),
    )
    .unwrap();
    let (_source, mut cancel) = cancellation();
    let result = http
        .fetch(
            &origin,
            Some(&credential),
            Instant::now() + Duration::from_secs(3),
            &mut cancel,
        )
        .await
        .unwrap();
    assert_eq!(result.final_url, format!("{outside}/final"));
    assert_eq!(task.join().unwrap().len(), 2);
    for trust in [false, true] {
        let (port, certificate, task) = tls_server(1, |_, _, _| {
            panic!("invalid TLS connection sent an HTTP request")
        });
        let origin = format!(
            "https://{}.invalid:{port}",
            if trust { "wrong-host" } else { "fixture" }
        );
        let mut http = WebHttp::with_resolver(
            WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap(),
            fixed_resolver(),
        );
        if trust {
            http = http.with_certificates(&[certificate]).unwrap();
        }
        let (_source, mut cancel) = cancellation();
        assert!(matches!(
            http.fetch(
                &origin,
                None,
                Instant::now() + Duration::from_secs(3),
                &mut cancel
            )
            .await,
            Err(DomainError::Unavailable(_))
        ));
        assert_eq!(task.join().unwrap(), vec!["TLS rejected before request"]);
    }
}

struct ToolFixture {
    _data: tempfile::TempDir,
    store: ag1_storage::Store,
    registry: ag1_core::tools::registry::Registry,
    scope: ag1_core::tools::registry::Scope,
}
impl ToolFixture {
    fn new() -> Self {
        use serde_json::json;
        let data = tempfile::tempdir().unwrap();
        let store = ag1_storage::Store::open(data.path()).unwrap();
        store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":data.path(),"timestamp":"2026-09-05T00:00:00.000Z"}).to_string().as_bytes()).unwrap();
        store
            .register_origin("fixture", "fixture", &[4; 32])
            .unwrap();
        let mut registry = ag1_core::tools::registry::Registry::builtins().unwrap();
        registry
            .activate(
                &["webFetch".into(), "webSearch".into()],
                &["webFetch".into(), "webSearch".into()].into(),
            )
            .unwrap();
        Self {
            _data: data,
            store,
            registry,
            scope: ag1_core::tools::registry::Scope {
                workspace: "fixture-workspace".into(),
                session: "fixture".into(),
                epoch: 1,
                revision: 1,
            },
        }
    }
    fn artifacts(&self, operation: &str) -> ag1_storage::context::ContextStore<'_> {
        ag1_storage::context::ContextStore::new(
            &self.store,
            ag1_core::SessionId::new("fixture").unwrap(),
            "fixture",
            operation,
            "2026-09-05T00:00:00.000Z",
            ag1_core::context::features::Features::default(),
        )
        .unwrap()
    }
    fn ticket(
        &self,
        operation: &str,
        tool: &str,
        args: serde_json::Value,
        target: &str,
        now: u64,
    ) -> ag1_core::tools::approval::AuthorizedCall {
        use ag1_core::tools::{
            approval::{Authorizer, ExecutionFence, Preflight, PreflightContext},
            policy::Policy,
        };
        let policy: Policy =
            serde_json::from_value(serde_json::json!({"mode":"interactive","cwd":"/fixture"}))
                .unwrap();
        let mut authorizer = Authorizer::new("web-tool-fixture").unwrap();
        let call = self
            .registry
            .prepare(operation, operation, tool, &args, true)
            .unwrap();
        match authorizer
            .preflight(
                call,
                PreflightContext {
                    scope: &self.scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: target,
                    now,
                    ttl: 1000,
                    untrusted_extensions: false,
                },
            )
            .unwrap()
        {
            Preflight::Ready(t) => t,
            Preflight::Approval(a) => authorizer
                .answer(
                    &a.binding,
                    true,
                    &self.registry,
                    ExecutionFence {
                        scope: &self.scope,
                        policy_revision: 1,
                        target_revision: target,
                        now,
                    },
                )
                .unwrap()
                .unwrap(),
            _ => panic!("unexpected fixture rejection"),
        }
    }
}
#[tokio::test]
async fn selected_fetch_content_types_docs_hints_and_complete_event_coverage_reach_native_results()
{
    let _case = WEB_CASE.lock().await;
    use ag1_core::tools::research;
    use ag1_platform::web_tools::{WebExecution, WebService};
    use serde_json::json;
    let fixture = ToolFixture::new();
    for (name, body, hint) in [
        (
            "local",
            "<html><head><title>Local Doc</title></head><body><main><p>Useful local fetch content for parsing.</p></main></body></html>",
            None,
        ),
        (
            "docs",
            "<html><head><title>useActionState – React</title></head><body><main><h1>useActionState</h1><p>`useActionState` is a React Hook that lets you update state with side effects using Actions.</p></main></body></html>",
            Some(json!({"intent":"docs_reference","richFetch":false})),
        ),
        (
            "events",
            "<html><head><title>Claude Code leak article</title></head><body><article><p>On March 31, 2026, Anthropic accidentally published Claude Code source maps to npm.</p><p>The package exposed internal TypeScript source code for the CLI.</p><p>Anthropic confirmed the packaging issue and republished a clean package.</p></article></body></html>",
            Some(json!({"intent":"current_events","richFetch":true,"authoritative":true})),
        ),
    ] {
        let (port, task) = server(1, move |_, _| {
            String::from_utf8(ok(body.as_bytes()))
                .unwrap()
                .replace("text/plain; charset=utf-8", "text/html; charset=utf-8")
                .into_bytes()
        });
        let origin = format!("http://127.0.0.1:{port}");
        let url = format!("{origin}/{name}");
        let service = WebService::new(
            WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap()),
            fixture.scope.clone(),
            name,
            Default::default(),
        )
        .unwrap();
        if let Some(hint) = hint {
            service
                .remember_hints(
                    &fixture.scope,
                    std::slice::from_ref(&url),
                    &serde_json::from_value(hint).unwrap(),
                    11,
                )
                .unwrap();
        }
        let ticket = fixture.ticket(
            name,
            "webFetch",
            json!({"url":url}),
            service.target_revision(),
            12,
        );
        let mut artifacts = fixture.artifacts(name);
        let (_source, mut cancel) = cancellation();
        let result = service
            .execute(
                ticket,
                WebExecution {
                    registry: &fixture.registry,
                    scope: &fixture.scope,
                    policy_revision: 1,
                    now: || 12,
                    deadline: Instant::now() + Duration::from_secs(3),
                    cancel: &mut cancel,
                    profile: research::profile(50000., 4.),
                },
                &mut artifacts,
            )
            .await
            .unwrap()
            .result;
        let text = result["content"][0]["text"].as_str().unwrap();
        assert_eq!(result["details"]["contentType"], "text/html; charset=utf-8");
        assert!(!text.contains("__CRUCIBLE_CONTENT_TYPE__"));
        assert!(!text.contains("text/html; charset=utf-8"));
        match name {
            "local" => assert!(text.contains("Useful local fetch content")),
            "docs" => assert_eq!(result["details"]["intent"], "docs_reference"),
            "events" => {
                assert_eq!(
                    result["details"]["coverage"]["present"],
                    json!(["date", "actor", "mechanism", "impact", "response"])
                );
                assert_eq!(result["details"]["coverage"]["missing"], json!([]));
                assert!(text.contains("Coverage complete for a brief current-events answer."));
                assert!(!text.contains("Reuse this URL as the source reference"));
            }
            _ => unreachable!(),
        }
        assert_eq!(task.join().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn selected_search_default_missing_brave_key_and_positive_lite_fallback_execute_native_http()
{
    let _case = WEB_CASE.lock().await;
    use ag1_core::tools::research;
    use ag1_platform::web_tools::{
        SearchConfig, SearchEndpoints, SearchService, WebExecution, WebService,
    };
    use serde_json::json;
    for (mode, fallback) in [("auto", false), ("brave", false), ("auto", true)] {
        let fixture = ToolFixture::new();
        let (port, task) = server(if fallback { 2 } else { 1 }, move |index, request| {
            assert!(request.starts_with(if index == 0 {
                "GET /html?"
            } else {
                "GET /lite?"
            }));
            if fallback && index == 0 {
                return ok(b"No results");
            }
            ok(b"<a class=\"result__a\" href=\"https://react.dev/reference/react\">React Reference</a><a class=\"result__snippet\">Official React reference docs.</a>")
        });
        let origin = format!("http://127.0.0.1:{port}");
        let web = Arc::new(
            WebService::new(
                WebHttp::new(
                    WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap(),
                ),
                fixture.scope.clone(),
                "selected-default-web",
                Default::default(),
            )
            .unwrap(),
        );
        let search = SearchService::new(
            web,
            SearchConfig {
                mode: mode.into(),
                brave: None,
                endpoints: SearchEndpoints {
                    html: format!("{origin}/html"),
                    lite: format!("{origin}/lite"),
                    ..SearchEndpoints::default()
                },
            },
            "selected-default-search",
        )
        .unwrap();
        let ticket = fixture.ticket(
            "search",
            "webSearch",
            json!({"query":"React reference"}),
            search.target_revision(),
            12,
        );
        let mut artifacts = fixture.artifacts("search");
        let (_source, mut cancel) = cancellation();
        let result = search
            .execute(
                ticket,
                WebExecution {
                    registry: &fixture.registry,
                    scope: &fixture.scope,
                    policy_revision: 1,
                    now: || 12,
                    deadline: Instant::now() + Duration::from_secs(3),
                    cancel: &mut cancel,
                    profile: research::profile(50000., 4.),
                },
                2026,
                &mut artifacts,
            )
            .await
            .unwrap()
            .result;
        let details = &result["details"];
        assert_eq!(details["searchProviderMode"], mode);
        assert_eq!(
            details["searchProviderOrder"],
            if mode == "brave" {
                json!(["brave", "duckduckgo"])
            } else {
                json!(["duckduckgo"])
            }
        );
        assert_eq!(
            details["searchProvider"],
            if fallback {
                "duckduckgo-lite"
            } else {
                "duckduckgo-html"
            }
        );
        assert_eq!(
            details["sourceUrls"][0],
            "https://react.dev/reference/react"
        );
        if mode == "brave" {
            assert_eq!(details["searchAttempts"][0]["status"], "skipped");
        }
        assert_eq!(task.join().unwrap().len(), if fallback { 2 } else { 1 });
    }
}

#[tokio::test]
async fn native_brave_search_sends_only_host_supplied_https_key_and_retains_provider_body() {
    let _case = WEB_CASE.lock().await;
    use ag1_core::{
        context::ContextArtifactPort,
        tools::{executor::ExecutionArtifacts, research},
    };
    use ag1_platform::web_tools::{
        SearchConfig, SearchEndpoints, SearchService, WebExecution, WebService,
    };
    use serde_json::json;
    let fixture = ToolFixture::new();
    let (port, certificate, task) = tls_server(1, |_, input, _| {
        let request = input.to_lowercase();
        assert!(request.contains("x-subscription-token: fixture-key"));
        assert!(request.contains("accept: application/json"));
        assert!(input.starts_with("GET /search?q=React%20parameter%20official%20docs&count=3 "));
        let body=json!({"web":{"results":[{"title":"React parameter reference","url":"https://react.dev/reference/react/useEffect","description":"The setup parameter returns a cleanup function."}]}}).to_string();
        let mut response = ok(body.as_bytes());
        let text = String::from_utf8(response)
            .unwrap()
            .replace("text/plain; charset=utf-8", "application/json");
        response = text.into_bytes();
        response
    });
    let origin = format!("https://brave.fixture.invalid:{port}");
    let http = WebHttp::with_resolver(
        WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap(),
        fixed_resolver(),
    )
    .with_certificates(&[certificate])
    .unwrap();
    let web = Arc::new(
        WebService::new(http, fixture.scope.clone(), "brave-web", Default::default()).unwrap(),
    );
    let search = SearchService::new(
        web,
        SearchConfig {
            mode: "auto".into(),
            endpoints: SearchEndpoints {
                brave: format!("{origin}/search"),
                ..SearchEndpoints::default()
            },
            brave: Some(
                WebCredential::new(
                    &["brave.fixture.invalid".into()],
                    "x-subscription-token",
                    &SecretBytes::new(b"fixture-key".to_vec()),
                )
                .unwrap(),
            ),
        },
        "brave-search",
    )
    .unwrap();
    for (operation, cache_hit) in [("brave-first", false), ("brave-cached", true)] {
        let ticket = fixture.ticket(
            operation,
            "webSearch",
            json!({"query":"React parameter"}),
            search.target_revision(),
            12,
        );
        let mut artifacts = fixture.artifacts(operation);
        let (_source, mut cancel) = cancellation();
        let result = search
            .execute(
                ticket,
                WebExecution {
                    registry: &fixture.registry,
                    scope: &fixture.scope,
                    policy_revision: 1,
                    now: || 12,
                    deadline: Instant::now() + Duration::from_secs(3),
                    cancel: &mut cancel,
                    profile: research::profile(90000., 0.),
                },
                2026,
                &mut artifacts,
            )
            .await
            .unwrap();
        assert_eq!(result.result["details"]["searchProvider"], "brave");
        assert_eq!(result.result["details"]["cacheHit"], cache_hit);
        assert_eq!(result.result["details"]["resultCount"], 1);
        assert!(!result.result.to_string().contains("fixture-key"));
        let id = result.result["details"]["networkEvidence"]["artifactIds"][0]
            .as_str()
            .unwrap();
        let raw = artifacts.read_json(&artifacts.get(id).unwrap()).unwrap();
        assert_eq!(raw["trust"], "untrusted");
        assert!(raw["bodyBase64"].as_str().unwrap().len() > 50);
        assert!(!raw.to_string().contains("fixture-key"));
    }
    assert_eq!(task.join().unwrap().len(), 1);
}
#[tokio::test]
async fn native_search_fallback_order_cache_dimensions_and_empty_ttl_are_executable() {
    let _case = WEB_CASE.lock().await;
    use ag1_core::tools::research;
    use ag1_platform::web_tools::{
        SearchConfig, SearchEndpoints, SearchService, WebExecution, WebService,
    };
    use serde_json::json;
    let fixture = ToolFixture::new();
    let (port, task) = server(6, |index, input| {
        assert!(input.to_lowercase().contains("user-agent: mozilla/5.0"));
        assert!(
            input
                .to_lowercase()
                .contains("accept: text/html,application/xhtml+xml")
        );
        if index < 2 {
            assert!(input.starts_with("GET /html?"));
            ok(b"<a class=\"result__a\" href=\"https://react.dev/reference/item\">React API</a><a class=\"result__snippet\">The parameter returns a useful value.</a><a class=\"result__a\" href=\"https://react.dev/learn/other\">Other API</a><a class=\"result__snippet\">Another useful source.</a>")
        } else {
            ok(b"No fixture results")
        }
    });
    let origin = format!("http://127.0.0.1:{port}");
    let web = Arc::new(
        WebService::new(
            WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap()),
            fixture.scope.clone(),
            "ddg-web",
            Default::default(),
        )
        .unwrap(),
    );
    let search = SearchService::new(
        web,
        SearchConfig {
            mode: "serpapi".into(),
            endpoints: SearchEndpoints {
                html: format!("{origin}/html"),
                lite: format!("{origin}/lite"),
                ..SearchEndpoints::default()
            },
            brave: None,
        },
        "ddg-search",
    )
    .unwrap();
    for (operation, query, max, now, cache, count) in [
        ("one", "React parameter", 1, 12, false, 1),
        ("more", "React parameter", 3, 13, false, 2),
        ("cached", "React parameter", 3, 14, true, 2),
        ("empty", "Unknown search fixture", 3, 15, false, 0),
        ("empty-cached", "Unknown search fixture", 3, 59999, true, 0),
        (
            "empty-expired",
            "Unknown search fixture",
            3,
            60016,
            false,
            0,
        ),
    ] {
        let mut artifacts = fixture.artifacts(operation);
        let ticket = fixture.ticket(
            operation,
            "webSearch",
            json!({"query":query,"maxResults":max}),
            search.target_revision(),
            now,
        );
        let (_source, mut cancel) = cancellation();
        let result = search
            .execute(
                ticket,
                WebExecution {
                    registry: &fixture.registry,
                    scope: &fixture.scope,
                    policy_revision: 1,
                    now: || now,
                    deadline: Instant::now() + Duration::from_secs(3),
                    cancel: &mut cancel,
                    profile: research::profile(90000., 0.),
                },
                2026,
                &mut artifacts,
            )
            .await
            .unwrap();
        let details = &result.result["details"];
        assert_eq!(details["cacheHit"], cache);
        assert_eq!(details["resultCount"], count);
        assert_eq!(
            details["searchProviderOrder"],
            json!(["serpapi", "duckduckgo"])
        );
        assert_eq!(details["searchAttempts"][0]["status"], "skipped");
        assert!(
            details["searchAttempts"][0]["error"]
                .as_str()
                .unwrap()
                .contains("not implemented yet")
        );
    }
    let requests = task.join().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(requests[3].starts_with("GET /lite?"));
    assert!(requests[5].starts_with("GET /lite?"));
}

#[tokio::test]
async fn stale_approval_foreign_epoch_and_precancelled_search_never_resolve_or_connect() {
    let _case = WEB_CASE.lock().await;
    use ag1_core::tools::research;
    use ag1_platform::web_tools::{SearchConfig, SearchService, WebExecution, WebService};
    use serde_json::json;
    let fixture = ToolFixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver = Resolver {
        calls: calls.clone(),
        hosts: Arc::new(Mutex::new(vec![])),
        answers: vec![vec!["1.1.1.1".parse().unwrap()]],
    };
    let web = Arc::new(
        WebService::new(
            WebHttp::with_resolver(WebPolicy::default(), resolver),
            fixture.scope.clone(),
            "fenced-web",
            Default::default(),
        )
        .unwrap(),
    );
    let search = SearchService::new(web, SearchConfig::default(), "current-search").unwrap();
    for kind in ["stale-target", "foreign-epoch", "cancelled", "expired"] {
        let target = if kind == "stale-target" {
            "stale-search-config"
        } else {
            search.target_revision()
        };
        let ticket = fixture.ticket(kind, "webSearch", json!({"query":"fixture"}), target, 12);
        let mut scope = fixture.scope.clone();
        if kind == "foreign-epoch" {
            scope.epoch += 1;
        }
        let (source, mut cancel) = cancellation();
        if kind == "cancelled" {
            source.cancel();
        }
        let deadline = if kind == "expired" {
            Instant::now() - Duration::from_secs(1)
        } else {
            Instant::now() + Duration::from_secs(1)
        };
        let mut artifacts = fixture.artifacts(kind);
        let result = search
            .execute(
                ticket,
                WebExecution {
                    registry: &fixture.registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    deadline,
                    cancel: &mut cancel,
                    profile: research::profile(90000., 0.),
                },
                2026,
                &mut artifacts,
            )
            .await;
        assert_eq!(
            result.err(),
            Some(match kind {
                "stale-target" => DomainError::Conflict,
                "foreign-epoch" => DomainError::PermissionDenied,
                "cancelled" => DomainError::Cancelled,
                _ => DomainError::TimedOut,
            })
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn active_search_cancellation_commits_one_no_effect_outcome_without_late_hints_or_retry() {
    let _case = WEB_CASE.blocking_lock();
    use ag1_core::{
        Clock,
        tools::{
            approval::ExecutionFence,
            executor::{self, Effect, Execution, NoHooks, Status},
            research,
            result::ResultBudget,
        },
    };
    use ag1_platform::web_tools::{
        SearchConfig, SearchEndpoints, SearchService, WebExecution, WebService,
    };
    use serde_json::json;
    struct Fixed;
    impl Clock for Fixed {
        fn now_ms(&self) -> u64 {
            12
        }
    }
    let fixture = ToolFixture::new();
    let (source, mut cancel) = cancellation();
    let (port, task) = server(1, move |_, _| {
        source.cancel();
        thread::sleep(Duration::from_millis(30));
        ok(b"late body")
    });
    let origin = format!("http://127.0.0.1:{port}");
    let web = Arc::new(
        WebService::new(
            WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap()),
            fixture.scope.clone(),
            "cancel-web",
            Default::default(),
        )
        .unwrap(),
    );
    let search = SearchService::new(
        web,
        SearchConfig {
            mode: "duckduckgo".into(),
            endpoints: SearchEndpoints {
                html: format!("{origin}/html"),
                lite: format!("{origin}/lite"),
                ..SearchEndpoints::default()
            },
            brave: None,
        },
        "cancel-search",
    )
    .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut artifacts = fixture.artifacts("cancel-search");
    let mut events = vec![];
    let mut execute = |ticket| {
        executor::execute_with_artifacts(
            &fixture.store,
            &mut artifacts,
            ticket,
            Execution {
                registry: &fixture.registry,
                fence: ExecutionFence {
                    scope: &fixture.scope,
                    policy_revision: 1,
                    target_revision: search.target_revision(),
                    now: 12,
                },
                budget: ResultBudget::default(),
                clock: &Fixed,
            },
            &mut NoHooks,
            |ticket, artifacts| {
                runtime.block_on(search.execute(
                    ticket,
                    WebExecution {
                        registry: &fixture.registry,
                        scope: &fixture.scope,
                        policy_revision: 1,
                        now: || 12,
                        deadline: Instant::now() + Duration::from_secs(3),
                        cancel: &mut cancel,
                        profile: research::profile(90000., 0.),
                    },
                    2026,
                    artifacts,
                ))
            },
            |event| events.push(event),
        )
    };
    let first = execute(fixture.ticket(
        "cancel-search",
        "webSearch",
        json!({"query":"fixture"}),
        search.target_revision(),
        12,
    ))
    .unwrap();
    assert_eq!(first.record.effect, Effect::None);
    assert_eq!(first.record.status, Status::Completed);
    assert_eq!(first.result["details"]["termination"], "cancelled");
    let replay = execute(fixture.ticket(
        "cancel-search",
        "webSearch",
        json!({"query":"fixture"}),
        search.target_revision(),
        12,
    ))
    .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.result, first.result);
    assert_eq!(events.len(), 2);
    assert_eq!(task.join().unwrap().len(), 1);
}

#[tokio::test]
async fn named_web_credentials_reject_unknown_and_foreign_hosts_before_dns_without_secret_disclosure()
 {
    let _case = WEB_CASE.lock().await;
    use ag1_core::tools::research;
    use ag1_platform::web_tools::{WebExecution, WebService};
    use serde_json::json;
    let (port, certificate, task) = tls_server(1, |_, request, _| {
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer fixture-private-token")
        );
        ok(b"Credential fixture response.")
    });
    let fixture = ToolFixture::new();
    let origin = format!("https://fixture.invalid:{port}");
    let resolver = fixed_resolver();
    let calls = resolver.calls.clone();
    let service = WebService::new(
        WebHttp::with_resolver(
            WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap(),
            resolver,
        )
        .with_certificates(&[certificate])
        .unwrap(),
        fixture.scope.clone(),
        "named-credentials",
        [(
            "github".into(),
            WebCredential::new(
                &["fixture.invalid".into()],
                "authorization",
                &SecretBytes::new(b"Bearer fixture-private-token".to_vec()),
            )
            .unwrap(),
        )]
        .into(),
    )
    .unwrap();
    for (id, credential, url, expected) in [
        ("unknown", "stripe", origin.clone(), "Available: github"),
        (
            "foreign",
            "github",
            "https://evil.example/x".into(),
            "evil.example",
        ),
        (
            "allowed",
            "github",
            origin.clone(),
            "Credential fixture response",
        ),
    ] {
        let ticket = fixture.ticket(
            id,
            "webFetch",
            json!({"url":url,"credential":credential}),
            service.target_revision(),
            12,
        );
        let (_source, mut cancel) = cancellation();
        let result = service
            .execute(
                ticket,
                WebExecution {
                    registry: &fixture.registry,
                    scope: &fixture.scope,
                    policy_revision: 1,
                    now: || 12,
                    deadline: Instant::now() + Duration::from_secs(3),
                    cancel: &mut cancel,
                    profile: research::profile(90000., 0.),
                },
                &mut fixture.artifacts(id),
            )
            .await
            .unwrap()
            .result;
        let text = result.to_string();
        assert!(text.contains(expected), "{id}: {text}");
        assert!(!text.contains("fixture-private-token"));
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(id == "allowed"));
        assert_eq!(result["details"].get("error").is_some(), id != "allowed");
    }
    assert_eq!(task.join().unwrap().len(), 1);
}
