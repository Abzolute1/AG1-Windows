//! Actual owned TLS upload bytes and uncertain response boundaries.
use super::*;
use crate::github::uploads::{Budget, fixture_source};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
#[test]
fn real_tls_upload_sends_exact_binary_to_one_origin_and_never_retries() {
    let (dir, root, grant, request) = fixture_source();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    let generated = crate::github::fixture_openssl()
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
    assert!(generated.status.success());
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
    let captured = Arc::new(Mutex::new(Vec::new()));
    let seen = captured.clone();
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
            let Ok(mut stream) = acceptor.accept(socket) else {
                continue;
            };
            let mut raw = Vec::new();
            let mut b = [0];
            while raw.len() < 16384 && !raw.ends_with(b"\r\n\r\n") {
                if stream.read(&mut b).unwrap_or(0) == 0 {
                    break;
                }
                raw.push(b[0]);
            }
            if raw.is_empty() {
                continue;
            }
            let headers = String::from_utf8(raw).unwrap();
            let length = headers
                .lines()
                .find_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            assert!(length < 1024);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            seen.lock().unwrap().push((headers.clone(), body));
            let route = headers.split_whitespace().nth(1).unwrap();
            let (status, extra, body) = if route.contains("?name=redirect.bin") {
                (
                    "302 Found",
                    "Location: https://evil.invalid/upload\r\n",
                    "{}".into(),
                )
            } else if route.contains("?name=denied.bin") {
                ("403 Forbidden", "", "{}".into())
            } else if route.contains("?name=partial.bin") {
                ("201 Created", "Content-Length: 100\r\n", "{".into())
            } else if route.contains("?name=conflict.bin") {
                ("422 Unprocessable Entity", "", "{}".into())
            } else {
                ("201 Created","",serde_json::json!({"id":99,"name":"owned.bin","size":5,"state":"uploaded","token":"synthetic-secret"}).to_string())
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
    let cancel = AtomicBool::new(false);
    let c = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let upload = Upload::capture(&request, root, grant, 1, &Budget::default(), &c).unwrap();
    let token = Token(b"synthetic-secret".to_vec());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for name in [
        "owned.bin",
        "redirect.bin",
        "denied.bin",
        "partial.bin",
        "conflict.bin",
    ] {
        let mut op = request.compile().unwrap();
        op.endpoint = op.endpoint.replace("owned.bin", name);
        let result = runtime.block_on(upload_to(&client, &token, &op, &upload, &c, &origin));
        if name == "owned.bin" {
            let r = result.unwrap();
            assert_eq!(r.outcome, Outcome::Applied);
            assert_eq!(r.data["token"], "[redacted]");
        } else {
            assert_eq!(result.unwrap_err().outcome, Outcome::Unknown);
        }
    }
    stop.store(true, Ordering::Release);
    worker.join().unwrap();
    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 5);
    for (header, bytes) in captured.iter() {
        assert_eq!(bytes, &[0, 255, 13, 10, 65]);
        assert!(
            header
                .to_lowercase()
                .contains("authorization: bearer synthetic-secret")
        );
        assert!(
            header
                .to_lowercase()
                .contains("content-type: application/octet-stream")
        );
    }
}
