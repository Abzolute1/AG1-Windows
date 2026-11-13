//! Owned TLS fixtures exercise the actual request/response path, with a private
//! test CA trusted only by this client. No gh, live account or external writes.
use super::*;
use serde_json::json;
use std::{
    io::Write,
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

struct Fixture {
    _dir: tempfile::TempDir,
    origin: String,
    client: Client,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        let generated = std::process::Command::new("/usr/bin/openssl")
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
                "subjectAltName=IP:127.0.0.1",
                "-keyout",
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .unwrap();
        assert!(
            generated.status.success(),
            "owned TLS certificate generation"
        );
        let pem = std::fs::read(cert).unwrap();
        let acceptor = native_tls::TlsAcceptor::new(
            native_tls::Identity::from_pkcs8(&pem, &std::fs::read(key).unwrap()).unwrap(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("https://{}", listener.local_addr().unwrap());
        let client = Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let halt = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let redirect_origin = origin.clone();
        let worker = thread::spawn(move || {
            while !halt.load(Ordering::Acquire) {
                let socket = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("owned TLS accept: {e}"),
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
                if raw.is_empty() {
                    continue;
                }
                let headers = String::from_utf8(raw.clone()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                assert!(length <= 128 * 1024);
                let mut body = vec![0; length];
                if stream.read_exact(&mut body).is_err() {
                    continue;
                }
                let request_body = String::from_utf8(body.clone()).unwrap();
                raw.extend(body);
                seen.lock().unwrap().push(String::from_utf8(raw).unwrap());
                let route = headers.split_whitespace().nth(1).unwrap();
                if route == "/hang" {
                    while !halt.load(Ordering::Acquire) {
                        thread::sleep(Duration::from_millis(2));
                    }
                    continue;
                }
                if route == "/binary-stream-large" {
                    let _=stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                    for _ in 0..1025 {
                        if stream
                            .write_all(b"10000\r\n")
                            .and_then(|_| stream.write_all(&[b'x'; 65536]))
                            .and_then(|_| stream.write_all(b"\r\n"))
                            .is_err()
                        {
                            break;
                        }
                    }
                    let _ = stream.write_all(b"0\r\n\r\n");
                    continue;
                }
                let redirect_header = format!("Location: {redirect_origin}/binary-data\r\n");
                let (status,extra,body)=match route {
                    "/binary-redirect"=>("302 Found",redirect_header.as_str(),"".into()),
                    "/binary-data"=>("200 OK","","\0\r\nOwned binary é".into()),
                    "/binary-large"=>("200 OK","Content-Length: 67108865\r\n","".into()),
                    "/permission"=>("403 Forbidden","",json!({"message":"synthetic-secret must not be reflected"}).to_string()),
                    "/rate"=>("429 Too Many Requests","Retry-After: 17\r\nX-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: 123456\r\n","{}".into()),
                    "/conflict"=>("409 Conflict","","{}".into()),
                    "/redirect"=>("302 Found","Location: https://untrusted.invalid/credential-trap\r\n","".into()),
                    "/malformed"=>("200 OK","","not JSON".into()),
                    "/oversized"=>("200 OK","Content-Length: 4194305\r\n","".into()),
                    "/partial"=>("200 OK","Content-Length: 100\r\n","{".into()),
                    "/merge"=>("200 OK","",json!({"merged":false,"message":"blocked"}).to_string()),
                    "/graphql" if request_body.contains("errors-fixture") => ("200 OK","",json!({"data":{"someMutation":{"id":"maybe-applied"}},"errors":[{"message":"synthetic-secret"}]}).to_string()),
                    "/graphql" if request_body.contains("bad-envelope") => ("200 OK","",json!({"errors":"not an array"}).to_string()),
                    _=>("200 OK","Link: <https://api.github.com/repos/owned/fixture/issues?page=2>; rel=\"next\"\r\nX-RateLimit-Remaining: 4999\r\nX-RateLimit-Limit: 5000\r\nX-RateLimit-Resource: core\r\n",json!({"id":1,"title":"<script>literal</script>","body":"synthetic-secret","token":"sensitive","synthetic-secret":"echoed key","data":{"rateLimit":{"cost":3,"remaining":4900,"resetAt":"2026-09-10T00:00:00Z"}}}).to_string()),
                };
                let length = if extra.contains("Content-Length:") {
                    String::new()
                } else {
                    format!("Content-Length: {}\r\n", body.len())
                };
                let _=stream.write_all(format!("HTTP/1.1 {status}\r\n{extra}{length}Connection: close\r\nContent-Type: application/json\r\n\r\n{body}").as_bytes());
                let _ = stream.shutdown();
            }
        });
        Self {
            _dir: dir,
            origin,
            client,
            stop,
            requests,
            worker: Some(worker),
        }
    }
    fn send(
        &self,
        id: &str,
        method: &str,
        path: &str,
        risk: Risk,
        body: Option<Value>,
    ) -> Result<Reply> {
        let cancel = AtomicBool::new(false);
        let control = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(3),
        };
        let operation = Operation {
            id: id.into(),
            method: method.into(),
            endpoint: path.into(),
            body,
            risk,
            repo: None,
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(send_to(
                &self.client,
                &Token(b"synthetic-secret".to_vec()),
                &operation,
                &control,
                &self.origin,
            ))
    }
}

#[test]
fn model_envelopes_reach_the_owned_tls_transport_with_exact_paths_and_bodies() {
    let fixture = Fixture::new();
    for args in [
        json!({"operation":"repos/get","parameters":{"owner":"owned","repo":"fixture"}}),
        json!({"operation":"repos/get-content","repo":"owned/fixture","parameters":{"path":"src/lib.rs"}}),
        json!({"operation":"issues/list","repo":"owned/fixture","parameters":{"page":2,"per_page":10}}),
        json!({"operation":"issues/create","repo":"owned/fixture","body":{"title":"Owned fixture"}}),
        json!({"operation":"pulls/get","repo":"owned/fixture","parameters":{"pull_number":7}}),
        json!({"operation":"actions/runs","repo":"owned/fixture"}),
        json!({"operation":"api.graphql","body":{"query":"query { viewer { login } }"}}),
    ] {
        let operation = super::super::tool::request(&args)
            .unwrap()
            .compile()
            .unwrap();
        super::super::schemas::validate(&operation).unwrap();
        let reply = fixture
            .send(
                &operation.id,
                &operation.method,
                &operation.endpoint,
                operation.risk,
                operation.body.clone(),
            )
            .unwrap();
        assert_eq!(reply.status, 200);
        let requests = fixture.requests.lock().unwrap();
        let request = requests.last().unwrap();
        assert!(request.starts_with(&format!(
            "{} /{} HTTP/1.1\r\n",
            operation.method, operation.endpoint
        )));
        if let Some(body) = operation.body {
            assert!(request.ends_with(&body.to_string()));
        }
    }
}

#[test]
fn real_tls_response_matrix_preserves_exact_requests_and_bounds_errors() {
    let fixture = Fixture::new();
    for (id, path) in [
        ("repos.get", "repos/owned/fixture"),
        ("issues.get", "repos/owned/fixture/issues/1"),
        ("pulls.get", "repos/owned/fixture/pulls/1"),
        ("actions.jobs", "repos/owned/fixture/actions/runs/1/jobs"),
        ("releases.assets", "repos/owned/fixture/releases/1/assets"),
        ("notifications.list", "notifications"),
        ("discussions.list", "graphql"),
        ("projects.items", "graphql"),
    ] {
        let method = if path == "graphql" { "POST" } else { "GET" };
        let reply = fixture
            .send(
                id,
                method,
                path,
                Risk::Read,
                (method == "POST").then(|| json!({"query":"owned fixture"})),
            )
            .unwrap();
        assert_eq!(reply.outcome, Outcome::Read);
        assert_eq!(reply.status, 200);
        assert_eq!(reply.rate_remaining, Some(4999));
        assert_eq!(reply.rate_resource.as_deref(), Some("core"));
        assert_eq!(reply.rate_limit, Some(5000));
        assert_eq!(reply.data["body"], "[redacted]");
        assert_eq!(reply.data["token"], "[redacted]");
        assert_eq!(reply.data["title"], "<script>literal</script>");
        assert!(
            !serde_json::to_string(&reply)
                .unwrap()
                .contains("synthetic-secret")
        );
        assert_eq!(
            reply.next.as_deref(),
            Some("repos/owned/fixture/issues?page=2")
        );
    }
    let body = json!({"title":"Exact owned issue","body":"literal\ntext"});
    assert_eq!(
        fixture
            .send(
                "issues.create",
                "POST",
                "repos/owned/fixture/issues",
                Risk::Write,
                Some(body.clone())
            )
            .unwrap()
            .outcome,
        Outcome::Applied
    );
    let request = fixture.requests.lock().unwrap().last().unwrap().clone();
    assert!(request.starts_with("POST /repos/owned/fixture/issues HTTP/1.1\r\n"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer synthetic-secret\r\n")
    );
    assert!(request.to_ascii_lowercase().contains(&format!(
        "x-github-api-version: {}\r\n",
        super::super::REST_VERSION
    )));
    assert_eq!(
        serde_json::from_str::<Value>(request.split_once("\r\n\r\n").unwrap().1).unwrap(),
        body
    );
    for (path, code, status) in [
        ("permission", "permission", Some(403)),
        ("rate", "rate_limit", Some(429)),
        ("conflict", "conflict", Some(409)),
        ("redirect", "redirect", Some(302)),
        ("malformed", "malformed", None),
        ("oversized", "response_limit", None),
        ("partial", "interrupted", None),
        ("merge", "merge_not_applied", None),
        ("graphql", "graphql", None),
    ] {
        let before = fixture.requests.lock().unwrap().len();
        let error = fixture
            .send(
                if path == "merge" {
                    "pulls.merge"
                } else {
                    "api.graphql"
                },
                "POST",
                path,
                Risk::Consequential,
                Some(json!({"query":"errors-fixture"})),
            )
            .unwrap_err();
        assert_eq!(error.code, code, "{path}");
        assert_eq!(error.status, status, "{path}");
        assert_eq!(error.outcome, Outcome::Unknown);
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("synthetic-secret")
        );
        if path == "rate" {
            assert_eq!(error.retry_after_seconds, Some(17));
            assert_eq!(error.retry_at_epoch_seconds, Some(123456));
        }
        assert_eq!(
            fixture.requests.lock().unwrap().len(),
            before + 1,
            "No retry or redirect for {path}"
        );
    }
    assert_eq!(
        fixture
            .send(
                "api.graphql",
                "POST",
                "graphql",
                Risk::Consequential,
                Some(json!({"query":"bad-envelope"}))
            )
            .unwrap_err()
            .code,
        "malformed_graphql"
    );
}

#[test]
fn real_tls_cancellation_discards_inflight_response_without_replay() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(3);
            while fixture.requests.lock().unwrap().is_empty() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            cancel.store(true, Ordering::Release);
        });
        let operation = Operation {
            id: "issues.create".into(),
            method: "POST".into(),
            endpoint: "hang".into(),
            body: Some(json!({"title":"Owned"})),
            risk: Risk::Write,
            repo: None,
        };
        let control = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(4),
        };
        let error = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(send_to(
                &fixture.client,
                &Token(b"synthetic-secret".to_vec()),
                &operation,
                &control,
                &fixture.origin,
            ))
            .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.outcome, Outcome::Unknown);
    });
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
}

#[test]
fn untrusted_tls_peer_never_receives_the_synthetic_credential() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(3),
    };
    let operation = Operation {
        id: "repos.get".into(),
        method: "GET".into(),
        endpoint: "repos/owned/fixture".into(),
        body: None,
        risk: Risk::Read,
        repo: None,
    };
    // Production client, without the fixture's private test CA.
    let error = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(send_to(
            &client().unwrap(),
            &Token(b"synthetic-secret".to_vec()),
            &operation,
            &control,
            &fixture.origin,
        ))
        .unwrap_err();
    assert_eq!(error.code, "network");
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[test]
fn binary_tls_transfer_keeps_signed_redirect_credentials_private_and_refuses_partial_bytes() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let token = Token(b"synthetic-secret".to_vec());
    // Native fixture only: production never admits loopback download locations.
    fn owned(raw: &str) -> Result<reqwest::Url> {
        let url = reqwest::Url::parse(raw).unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        Ok(url)
    }
    let op = |path: &str| Operation {
        id: "actions.download_logs".into(),
        method: "GET".into(),
        endpoint: path.into(),
        body: None,
        risk: Risk::Read,
        repo: Some("owned/repo".into()),
    };
    let payload = runtime
        .block_on(downloads::download_with(
            &fixture.client,
            &token,
            &op("binary-redirect"),
            &control,
            &fixture.origin,
            owned,
        ))
        .unwrap();
    assert_eq!(payload.bytes, b"\0\r\nOwned binary \xc3\xa9");
    let seen = fixture.requests.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert!(
        seen[0]
            .to_lowercase()
            .contains("authorization: bearer synthetic-secret")
    );
    assert!(!seen[1].to_lowercase().contains("authorization"));
    assert!(!seen[1].contains("synthetic-secret"));
    for (path, code) in [
        ("binary-large", "download_limit"),
        ("binary-stream-large", "download_limit"),
        ("partial", "download_interrupted"),
        ("permission", "permission"),
        ("rate", "rate_limit"),
        ("other", "download_credentials"),
    ] {
        let result = runtime.block_on(downloads::download_with(
            &fixture.client,
            &token,
            &op(path),
            &control,
            &fixture.origin,
            owned,
        ));
        assert_eq!(result.err().unwrap().code, code);
    }
    cancel.store(true, Ordering::Release);
    let count = fixture.requests.lock().unwrap().len();
    assert!(
        runtime
            .block_on(downloads::download_with(
                &fixture.client,
                &token,
                &op("binary-data"),
                &control,
                &fixture.origin,
                owned
            ))
            .is_err()
    );
    assert_eq!(fixture.requests.lock().unwrap().len(), count);
    cancel.store(false, Ordering::Release);
    thread::scope(|threads| {
        threads.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(3);
            while fixture.requests.lock().unwrap().len() == count {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(2));
            }
            cancel.store(true, Ordering::Release);
        });
        let result = runtime.block_on(downloads::download_with(
            &fixture.client,
            &token,
            &op("hang"),
            &control,
            &fixture.origin,
            owned,
        ));
        assert_eq!(result.err().unwrap().code, "cancelled");
    });
}
