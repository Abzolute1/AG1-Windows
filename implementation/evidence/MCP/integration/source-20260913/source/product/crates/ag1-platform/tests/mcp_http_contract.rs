use ag1_core::{DomainError, PortFuture};
use ag1_platform::{
    mcp::{
        Revision,
        catalog::{CatalogBinding, CatalogScope},
        discovery::{CatalogKind, DiscoveryLimits, Health},
        http::{Endpoint, HttpDiscovery, Shutdown},
    },
    provider::http::cancellation,
    web_http::WebResolver,
};
use serde_json::{Value, json};
#[path = "mcp_http/authenticated.rs"]
mod authenticated;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{IpAddr, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Clone)]
struct Seen {
    verb: String,
    headers: BTreeMap<String, String>,
    body: Value,
}
struct Fixture {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
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
trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}
impl Fixture {
    fn new(mode: &str) -> Self {
        Self::start(mode, None)
    }
    fn start(mode: &str, tls: Option<native_tls::TlsAcceptor>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "{}://{}/mcp",
            if tls.is_some() { "https" } else { "http" },
            listener.local_addr().unwrap()
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let copy = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let mode = mode.to_owned();
        let worker = thread::spawn(move || {
            let mut workers = vec![];
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        socket
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let seen = copy.clone();
                        let mode = mode.clone();
                        let stop = stopped.clone();
                        let tls = tls.clone();
                        workers.push(thread::spawn(move || {
                            let mut stream: Box<dyn Stream> = match tls {
                                None => Box::new(socket),
                                Some(tls) => match tls.accept(socket) {
                                    Ok(s) => Box::new(s),
                                    Err(_) => return,
                                },
                            };
                            let _ = serve(&mut *stream, &mode, &seen, &stop);
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("fixture accept: {e}"),
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
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
    fn client(&self, revision: Revision) -> HttpDiscovery {
        self.timed_client(revision, Duration::from_secs(3))
    }
    fn timed_client(&self, revision: Revision, time: Duration) -> HttpDiscovery {
        HttpDiscovery::new(
            Endpoint::loopback_development(&self.url).unwrap(),
            binding(revision),
            DiscoveryLimits::new(time).unwrap(),
        )
        .unwrap()
    }
}
fn binding(protocol: Revision) -> CatalogBinding {
    CatalogBinding {
        server_id: "26fb3347-af67-4a1d-92ec-a915db39c5d0".into(),
        scope: CatalogScope::GeneralChat,
        config_revision: 1,
        auth_profile: "owned-fixture".into(),
        credential_generation: 0,
        connection_generation: 1,
        protocol,
    }
}
fn respond(
    s: &mut dyn Stream,
    code: u16,
    mime: &str,
    headers: &str,
    body: &[u8],
) -> std::io::Result<()> {
    write!(
        s,
        "HTTP/1.1 {code} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    )?;
    s.write_all(body)?;
    s.flush()
}
fn serve(
    s: &mut dyn Stream,
    mode: &str,
    seen: &Mutex<Vec<Seen>>,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    let mut header = vec![];
    while !header.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        s.read_exact(&mut byte)?;
        header.push(byte[0]);
        assert!(header.len() < 32768);
    }
    let text = String::from_utf8(header).unwrap();
    let mut lines = text.split("\r\n");
    let verb = lines
        .next()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|s| s.split_once(':'))
        .map(|(k, v)| (k.to_lowercase(), v.trim().to_owned()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .map(|s| s.parse().unwrap())
        .unwrap_or(0);
    assert!(length < 8192);
    let mut body = vec![0; length];
    s.read_exact(&mut body)?;
    let body: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    seen.lock().unwrap().push(Seen {
        verb: verb.clone(),
        headers: headers.clone(),
        body: body.clone(),
    });
    if verb == "DELETE" {
        return respond(
            s,
            if mode == "no_delete" { 405 } else { 204 },
            "application/json",
            "",
            b"",
        );
    }
    assert_eq!(verb, "POST");
    if mode == "hang" {
        while !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
        return Ok(());
    }
    if let Some(location) = mode.strip_prefix("redirect:") {
        return respond(
            s,
            307,
            "application/json",
            &format!("Location: {location}\r\n"),
            b"",
        );
    }
    if let Some(code) = mode.strip_prefix("status:") {
        return respond(
            s,
            code.parse().unwrap(),
            "application/json",
            "",
            b"synthetic-private-error",
        );
    }
    let method = body["method"].as_str().unwrap_or("");
    let legacy = headers["mcp-protocol-version"] == "2025-11-25";
    assert_eq!(headers["accept"], "application/json, text/event-stream");
    assert!(
        !headers.contains_key("authorization")
            && !headers.contains_key("cookie")
            && !headers.contains_key("proxy-authorization")
    );
    if !legacy {
        assert_eq!(headers["mcp-method"], method);
        assert_eq!(
            body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert!(!headers.contains_key("mcp-session-id"));
    } else if method != "initialize" {
        assert_eq!(headers["mcp-session-id"], "owned-private-session");
    }
    if method == "notifications/initialized"
        || method == "notifications/cancelled"
        || method.is_empty()
    {
        return respond(
            s,
            202,
            "application/json",
            "",
            if mode == "bad_ack" {
                b"unexpected"
            } else {
                b""
            },
        );
    }
    if mode == "expired" && method != "initialize" {
        return respond(s, 404, "application/json", "", b"{}");
    }
    if mode == "slow_pages" && method == "tools/list" {
        thread::sleep(Duration::from_millis(90));
    }
    if mode == "stream_wait" && method == "tools/list" {
        s.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        )?;
        while !stop.load(Ordering::Acquire) {
            s.write_all(b": waiting\n\n")?;
            s.flush()?;
            thread::sleep(Duration::from_millis(10));
        }
        return Ok(());
    }
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{},"resources":{},"prompts":{}},"serverInfo":{"name":"fixture","version":"1"}})
        }
        "server/discover" => {
            json!({"resultType":"complete","supportedVersions":["2026-07-28"],"ttlMs":0,"cacheScope":"private","capabilities":{"tools":{},"resources":{},"prompts":{}}})
        }
        "tools/list" => {
            let second = body["params"].get("cursor").is_some();
            let mut result = json!({"tools":[{"name":if second{"second"}else{"first"},"inputSchema":{"type":"object"}}]});
            if !second {
                result["nextCursor"] = json!("next");
            }
            result
        }
        "resources/list" => json!({"resources":[{"name":"source","uri":"file:///not-authorized"}]}),
        "resources/templates/list" => {
            json!({"resourceTemplates":[{"name":"by-id","uriTemplate":"fixture://{id}"}]})
        }
        "prompts/list" => {
            json!({"prompts":[{"name":"review","arguments":[{"name":"topic","required":true}]}]})
        }
        _ => panic!("unexpected effecting method {method}"),
    };
    let id = if mode == "wrong_id" {
        json!("foreign")
    } else {
        body["id"].clone()
    };
    let message = json!({"jsonrpc":"2.0","id":id,"result":result});
    let encoded = serde_json::to_vec(&message).unwrap();
    let mut extra = if legacy && method == "initialize" {
        "Mcp-Session-Id: owned-private-session\r\nSet-Cookie: synthetic=private\r\n".to_owned()
    } else {
        String::new()
    };
    if mode == "session_swap" && method == "tools/list" {
        extra = "Mcp-Session-Id: substituted\r\n".into();
    }
    if mode == "session_current" && !legacy {
        extra = "Mcp-Session-Id: unexpected\r\n".into();
    }
    if mode == "duplicate_session" && method == "initialize" {
        extra.push_str("Mcp-Session-Id: second\r\n");
    }
    if mode == "compressed" {
        extra.push_str("Content-Encoding: gzip\r\n");
    }
    if mode == "oversized" {
        return write!(
            s,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 9000000\r\n\r\n"
        );
    }
    if mode == "duplicate_key" {
        return respond(
            s,
            200,
            "application/json",
            &extra,
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"id\":{},\"result\":{{}}}}",
                body["id"], body["id"]
            )
            .as_bytes(),
        );
    }
    if mode == "wrong_mime" {
        return respond(s, 200, "text/html", &extra, &encoded);
    }
    if [
        "sse",
        "changed",
        "server_request",
        "duplicate_response",
        "flood",
        "early_close",
        "sse_oversized",
    ]
    .contains(&mode)
    {
        let mut data = b": keepalive\r\n\r\n".to_vec();
        if mode == "changed" && method == "tools/list" {
            data.extend_from_slice(
                b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
            );
        }
        if mode == "server_request" && method == "tools/list" {
            data.extend_from_slice(b"data: {\"jsonrpc\":\"2.0\",\"id\":\"peer\",\"method\":\"sampling/createMessage\",\"params\":{}}\n\n");
        }
        if mode == "flood" {
            data.extend_from_slice(&b"\n".repeat(129));
        }
        if mode == "sse_oversized" {
            data.extend_from_slice(&vec![b'x'; 1024 * 1024 + 1]);
        }
        if mode == "early_close" {
            data.extend_from_slice(b"data: {\"jsonrpc\":");
        } else {
            data.extend_from_slice(b"event: message\ndata: ");
            data.extend_from_slice(&encoded);
            data.extend_from_slice(b"\n\n");
        }
        if mode == "duplicate_response" {
            data.extend_from_slice(b"data: ");
            data.extend_from_slice(&encoded);
            data.extend_from_slice(b"\n\n");
        }
        return respond(s, 200, "text/event-stream; charset=utf-8", &extra, &data);
    }
    respond(s, 200, "application/json; charset=utf-8", &extra, &encoded)
}

#[tokio::test]
async fn json_and_sse_discover_both_revisions_with_exact_sessions_and_headers() {
    for revision in [Revision::Legacy20251125, Revision::Current20260728] {
        for mode in ["json", "sse"] {
            let f = Fixture::new(mode);
            let mut c = f.client(revision);
            let (_source, mut cancel) = cancellation();
            assert!(f.requests().is_empty());
            c.negotiate(&mut cancel).await.unwrap();
            c.refresh_tools(&mut cancel).await.unwrap();
            assert_eq!(c.tools().unwrap().tools().count(), 2);
            for kind in [
                CatalogKind::Resources,
                CatalogKind::ResourceTemplates,
                CatalogKind::Prompts,
            ] {
                assert_eq!(c.list(kind, &mut cancel).await.unwrap().entries.len(), 1);
            }
            let result = c.shutdown(&mut cancel).await.unwrap();
            assert_eq!(
                result,
                if revision == Revision::Legacy20251125 {
                    Shutdown::RemoteSessionDeleted
                } else {
                    Shutdown::LocalOnly
                }
            );
            assert_eq!(c.health(), Health::Stopped);
            assert!(c.tools().is_none());
            let requests = f.requests();
            assert_eq!(
                requests.iter().filter(|r| r.verb == "DELETE").count(),
                usize::from(revision == Revision::Legacy20251125)
            );
            assert!(!requests.iter().any(|r| r.verb == "GET"));
        }
    }
}
#[tokio::test]
async fn redirects_never_contact_the_advertised_destination() {
    let target = Fixture::new("json");
    let source = Fixture::new(&format!("redirect:{}", target.url));
    let mut c = source.client(Revision::Current20260728);
    let (_s, mut t) = cancellation();
    assert!(matches!(
        c.negotiate(&mut t).await,
        Err(DomainError::PermissionDenied)
    ));
    assert!(target.requests().is_empty());
    assert_eq!(source.requests().len(), 1);
}
#[tokio::test]
async fn statuses_are_redacted_and_never_automatically_retried() {
    for code in [401, 403, 404, 429, 500] {
        let f = Fixture::new(&format!("status:{code}"));
        let mut c = f.client(Revision::Current20260728);
        let (_s, mut t) = cancellation();
        let error = c.negotiate(&mut t).await.unwrap_err();
        assert!(!format!("{error:?}").contains("synthetic-private-error"));
        assert_eq!(f.requests().len(), 1);
        assert_eq!(c.health(), Health::Failed);
    }
}
#[tokio::test]
async fn malformed_http_json_sse_and_current_sessions_fail_closed() {
    for mode in [
        "wrong_id",
        "compressed",
        "oversized",
        "duplicate_key",
        "wrong_mime",
        "duplicate_response",
        "early_close",
        "flood",
        "sse_oversized",
        "session_current",
    ] {
        let f = Fixture::new(mode);
        let mut c = f.client(Revision::Current20260728);
        let (_s, mut t) = cancellation();
        assert!(c.negotiate(&mut t).await.is_err(), "{mode}");
        assert_eq!(c.health(), Health::Failed);
        assert!(c.server_metadata().is_none());
        assert_eq!(f.requests().len(), 1);
    }
}
#[tokio::test]
async fn session_replacement_bad_ack_and_expiry_do_not_reinitialize() {
    for mode in ["session_swap", "expired", "duplicate_session", "bad_ack"] {
        let f = Fixture::new(mode);
        let mut c = f.client(Revision::Legacy20251125);
        let (_s, mut t) = cancellation();
        let result = c.negotiate(&mut t).await;
        if ["session_swap", "expired"].contains(&mode) {
            result.unwrap();
            assert!(c.refresh_tools(&mut t).await.is_err());
        } else {
            assert!(result.is_err());
        }
        assert_eq!(c.health(), Health::Failed);
        assert_eq!(
            f.requests()
                .iter()
                .filter(|r| r.body["method"] == "initialize")
                .count(),
            1
        );
    }
}
#[tokio::test]
async fn changed_tools_and_current_server_requests_do_not_publish() {
    for mode in ["changed", "server_request"] {
        let f = Fixture::new(mode);
        let mut c = f.client(Revision::Current20260728);
        let (_s, mut t) = cancellation();
        c.negotiate(&mut t).await.unwrap();
        assert!(c.refresh_tools(&mut t).await.is_err());
        assert!(c.tools().is_none());
        assert!(f.requests().iter().all(|r| r.body.get("error").is_none()));
    }
}
#[tokio::test]
async fn legacy_server_requests_receive_explicit_refusal_on_same_endpoint() {
    let f = Fixture::new("server_request");
    let mut c = f.client(Revision::Legacy20251125);
    let (_s, mut t) = cancellation();
    c.negotiate(&mut t).await.unwrap();
    c.refresh_tools(&mut t).await.unwrap();
    let callbacks: Vec<_> = f
        .requests()
        .into_iter()
        .filter(|r| r.body.get("error").is_some())
        .collect();
    assert_eq!(callbacks.len(), 2);
    for reply in callbacks {
        assert_eq!(reply.body["error"]["code"], -32601);
        assert_eq!(reply.headers["mcp-session-id"], "owned-private-session");
    }
    c.shutdown(&mut t).await.unwrap();
}
#[tokio::test]
async fn cancellation_before_send_during_wait_and_future_drop_retire_state() {
    let f = Fixture::new("hang");
    let mut c = f.client(Revision::Current20260728);
    let (source, mut t) = cancellation();
    source.cancel();
    assert!(matches!(
        c.negotiate(&mut t).await,
        Err(DomainError::Cancelled)
    ));
    assert!(f.requests().is_empty());
    let mut c = f.client(Revision::Current20260728);
    let (source, mut t) = cancellation();
    let signal = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        source.cancel();
    };
    let (result, ()) = tokio::join!(c.negotiate(&mut t), signal);
    assert!(matches!(result, Err(DomainError::Cancelled)));
    assert_eq!(c.health(), Health::Failed);
    let mut c = f.client(Revision::Current20260728);
    let (_s, mut t) = cancellation();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), c.negotiate(&mut t))
            .await
            .is_err()
    );
    assert_eq!(c.health(), Health::Failed);
    assert!(c.tools().is_none());
}
#[tokio::test]
async fn deadline_covers_every_page_and_connection_isolation_survives_cancel() {
    let f = Fixture::new("slow_pages");
    let mut c = f.timed_client(Revision::Current20260728, Duration::from_millis(150));
    let (_s, mut t) = cancellation();
    c.negotiate(&mut t).await.unwrap();
    assert!(matches!(
        c.refresh_tools(&mut t).await,
        Err(DomainError::TimedOut)
    ));
    assert!(c.tools().is_none());
    let other = Fixture::new("json");
    let mut client = other.client(Revision::Current20260728);
    client.negotiate(&mut t).await.unwrap();
    client.refresh_tools(&mut t).await.unwrap();
    assert_eq!(client.tools().unwrap().tools().count(), 2);
}
#[tokio::test]
async fn deletion_unsupported_is_distinct_from_confirmed_remote_deletion() {
    let f = Fixture::new("no_delete");
    let mut c = f.client(Revision::Legacy20251125);
    let (_s, mut t) = cancellation();
    c.negotiate(&mut t).await.unwrap();
    assert_eq!(
        c.shutdown(&mut t).await.unwrap(),
        Shutdown::RemoteDeletionUnsupported
    );
    assert_eq!(c.shutdown(&mut t).await.unwrap(), Shutdown::LocalOnly);
}

#[tokio::test]
async fn stream_cancellation_uses_each_revisions_own_semantics() {
    for revision in [Revision::Legacy20251125, Revision::Current20260728] {
        let f = Fixture::new("stream_wait");
        let mut c = f.client(revision);
        let (source, mut token) = cancellation();
        c.negotiate(&mut token).await.unwrap();
        let signal = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            source.cancel();
        };
        let (result, ()) = tokio::join!(c.refresh_tools(&mut token), signal);
        assert!(matches!(result, Err(DomainError::Cancelled)));
        assert!(c.tools().is_none());
        assert_eq!(c.health(), Health::Failed);
        let requests = f.requests();
        let cancellations: Vec<_> = requests
            .iter()
            .filter(|r| r.body["method"] == "notifications/cancelled")
            .collect();
        assert_eq!(
            cancellations.len(),
            usize::from(revision == Revision::Legacy20251125)
        );
        if let Some(cancellation) = cancellations.first() {
            assert_eq!(cancellation.body["params"]["requestId"], "ag1-mcp-2");
        }
    }
}

struct Resolver(Vec<IpAddr>);
impl WebResolver for Resolver {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> PortFuture<'a, Vec<IpAddr>> {
        Box::pin(async { Ok(self.0.clone()) })
    }
}
#[tokio::test]
async fn public_endpoints_reject_private_and_mixed_dns_before_connecting() {
    for addresses in [
        vec!["127.0.0.1"],
        vec!["8.8.8.8", "10.0.0.1"],
        vec!["::ffff:127.0.0.1"],
    ] {
        let endpoint = Endpoint::public_https("https://owned-fixture.invalid/mcp").unwrap();
        let mut c = HttpDiscovery::with_resolver(
            endpoint,
            binding(Revision::Current20260728),
            DiscoveryLimits::default(),
            Resolver(addresses.iter().map(|s| s.parse().unwrap()).collect()),
        )
        .unwrap();
        let (_s, mut t) = cancellation();
        assert!(matches!(
            c.negotiate(&mut t).await,
            Err(DomainError::PermissionDenied)
        ));
    }
}
#[test]
fn endpoint_configuration_is_inert_and_rejects_credential_or_private_expansion() {
    for url in [
        "http://public.example/mcp",
        "https://u:p@example.com/mcp",
        "https://example.com/mcp?token=secret",
        "https://example.com/mcp#fragment",
        "https://example.com/\n",
    ] {
        assert!(Endpoint::public_https(url).is_err());
    }
    for url in [
        "http://localhost/mcp",
        "http://10.0.0.1/mcp",
        "http://169.254.169.254/mcp",
    ] {
        assert!(Endpoint::loopback_development(url).is_err());
    }
}
#[tokio::test]
async fn https_requires_a_trusted_chain_even_on_explicit_loopback() {
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
    assert!(output.status.success());
    let cert = std::fs::read(cert).unwrap();
    let identity = native_tls::Identity::from_pkcs8(&cert, &std::fs::read(key).unwrap()).unwrap();
    let f = Fixture::start(
        "json",
        Some(native_tls::TlsAcceptor::new(identity).unwrap()),
    );
    let (_s, mut t) = cancellation();
    assert!(
        f.client(Revision::Current20260728)
            .negotiate(&mut t)
            .await
            .is_err()
    );
    assert!(f.requests().is_empty());
    let mut trusted = f
        .client(Revision::Current20260728)
        .with_certificates(&[cert])
        .unwrap();
    trusted.negotiate(&mut t).await.unwrap();
    assert_eq!(trusted.health(), Health::Ready);
}

#[cfg(target_os = "linux")]
#[test]
fn manager_remote_review_scope_catalog_and_owned_shutdown_cover_both_revisions() {
    use ag1_platform::mcp::{
        definitions::{Availability, Configuration, Draft, Provenance},
        manager::{ConnectionConsent, ConnectionHealth, Manager},
    };
    for (revision, mode) in [
        (Revision::Legacy20251125, "json"),
        (Revision::Current20260728, "json"),
        (Revision::Legacy20251125, "sse"),
        (Revision::Current20260728, "sse"),
    ] {
        let fixture = Fixture::new(mode);
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::open(
            ag1_storage::Store::open(dir.path()).unwrap(),
            std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
            DiscoveryLimits::new(Duration::from_secs(3)).unwrap(),
        )
        .unwrap();
        let server = manager
            .add_drafts(vec![Draft {
                name: "remote owned fixture".into(),
                configuration: Configuration::Http {
                    url: fixture.url.clone(),
                    loopback_development: true,
                },
                protocol: revision.as_str().into(),
                auth_profile: None,
                provenance: Provenance::Manual,
            }])
            .unwrap()
            .remove(0);
        assert!(fixture.requests().is_empty());
        assert!(
            manager
                .review_connection(&server, CatalogScope::GeneralChat)
                .is_err()
        );
        manager
            .set_availability(
                &server,
                1,
                Availability {
                    enabled: true,
                    general_chat: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let wrong = manager
            .review_connection(&server, CatalogScope::GeneralChat)
            .unwrap();
        assert!(
            manager
                .connect(wrong, ConnectionConsent::RemoteEndpoint)
                .is_err()
        );
        assert!(fixture.requests().is_empty());
        let review = manager
            .review_connection(&server, CatalogScope::GeneralChat)
            .unwrap();
        let connection = manager
            .connect(review, ConnectionConsent::LoopbackDevelopmentEndpoint)
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status = manager.status(&connection).unwrap();
            if status.health == ConnectionHealth::Ready {
                break;
            }
            assert!(
                status.health != ConnectionHealth::Failed && std::time::Instant::now() < deadline,
                "{status:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let catalog = manager
            .catalog(&connection, &CatalogScope::GeneralChat)
            .unwrap();
        assert_eq!(catalog.binding.protocol, revision);
        assert!(!catalog.tools.is_empty());
        assert!(!catalog.resources.is_empty());
        assert!(!catalog.templates.is_empty());
        assert!(!catalog.prompts.is_empty());
        assert!(
            manager
                .catalog(&connection, &CatalogScope::Workspace("foreign".into()))
                .is_err()
        );
        manager.disconnect(&connection).unwrap();
        assert!(
            manager
                .catalog(&connection, &CatalogScope::GeneralChat)
                .is_err()
        );
        manager.wait_disconnected(&connection).unwrap();
        assert_eq!(
            fixture
                .requests()
                .iter()
                .filter(|r| r.verb == "DELETE")
                .count(),
            usize::from(revision == Revision::Legacy20251125)
        );
        assert!(fixture.requests().iter().all(|r| {
            !["tools/call", "resources/read", "prompts/get"]
                .contains(&r.body["method"].as_str().unwrap_or_default())
        }));
    }
}
