use super::*;
use crate::github::secrets::Secret;
use base64::Engine;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
#[link(name = "sodium")]
unsafe extern "C" {
    fn sodium_init() -> i32;
    fn crypto_box_keypair(pk: *mut u8, sk: *mut u8) -> i32;
    fn crypto_box_seal_open(
        m: *mut u8,
        c: *const u8,
        len: u64,
        pk: *const u8,
        sk: *const u8,
    ) -> i32;
}
#[test]
fn actual_tls_secret_write_is_encrypted_and_never_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
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
                "subjectAltName=IP:127.0.0.1",
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
    let client = Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let halt = Arc::new(AtomicBool::new(false));
    let stop = halt.clone();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let seen = captured.clone();
    let worker = thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
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
            let header = String::from_utf8(raw).unwrap();
            let len = header
                .lines()
                .find_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            assert!(len < 1000);
            let mut body = vec![0; len];
            stream.read_exact(&mut body).unwrap();
            seen.lock().unwrap().push((header.clone(), body));
            let route = header.split_whitespace().nth(1).unwrap();
            let response = if route.ends_with("/DENIED") {
                "403 Forbidden\r\nContent-Length: 0\r\n"
            } else if route.ends_with("/REDIRECT") {
                "302 Found\r\nLocation: https://evil.invalid/secret\r\nContent-Length: 0\r\n"
            } else if route.ends_with("/PARTIAL") {
                "201 Created\r\nContent-Length: 100\r\n"
            } else if route.ends_with("/WRONG") {
                "200 OK\r\nContent-Length: 0\r\n"
            } else {
                "204 No Content\r\nContent-Length: 0\r\n"
            };
            let _ = stream
                .write_all(format!("HTTP/1.1 {response}Connection: close\r\n\r\n").as_bytes());
            let _ = stream.shutdown();
        }
    });
    let mut pk = [0; 32];
    let mut sk = [0; 32];
    unsafe {
        assert!(sodium_init() >= 0);
        assert_eq!(crypto_box_keypair(pk.as_mut_ptr(), sk.as_mut_ptr()), 0);
    }
    let secret = Secret::new(b"owned-fixture-private-value".to_vec()).unwrap();
    let key = serde_json::json!({"key_id":"owned","key":base64::engine::general_purpose::STANDARD.encode(pk)});
    let cancel = AtomicBool::new(false);
    let c = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let token = Token(b"synthetic-token".to_vec());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for name in ["OWNED", "DENIED", "REDIRECT", "PARTIAL", "WRONG"] {
        let request:crate::github::operations::Request=serde_json::from_value(serde_json::json!({"operation":"secrets.set","repo":"owned/repo","parameters":{"target":"actions_repository","name":name}})).unwrap();
        let op = request.compile().unwrap();
        let body = encrypted_body(&op, &secret, &key).unwrap();
        let result = rt.block_on(set_to(&client, &token, &op, body, &c, &origin));
        if name == "OWNED" {
            let r = result.unwrap();
            assert_eq!(r.data["secret_saved"], true);
            assert!(!r.data.to_string().contains("encrypted_value"));
        } else {
            assert_eq!(result.unwrap_err().outcome, Outcome::Unknown);
        }
    }
    halt.store(true, Ordering::Release);
    worker.join().unwrap();
    let rows = captured.lock().unwrap();
    assert_eq!(rows.len(), 5);
    for (headers, body) in rows.iter() {
        assert!(
            headers
                .to_lowercase()
                .contains("authorization: bearer synthetic-token")
        );
        assert!(!String::from_utf8_lossy(body).contains("owned-fixture-private-value"));
        let v: Value = serde_json::from_slice(body).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(v["encrypted_value"].as_str().unwrap())
            .unwrap();
        let mut plain = vec![0; bytes.len() - 48];
        unsafe {
            assert_eq!(
                crypto_box_seal_open(
                    plain.as_mut_ptr(),
                    bytes.as_ptr(),
                    bytes.len() as u64,
                    pk.as_ptr(),
                    sk.as_ptr()
                ),
                0
            );
        }
        assert_eq!(plain, b"owned-fixture-private-value");
    }
}
