//! A real native Cargo check against an isolated HTTPS sparse registry.
//! The test CA exists only in a child mount namespace, never the host trust store.
use ag1_platform::{
    projects::{Launch, Runtime, Toolchains},
    resource_fs::ScanControl,
    workspace::WorkspaceService,
};
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct RegistryServer {
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
    port: u16,
}
impl Drop for RegistryServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
fn certificate(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let cert = root.join(format!("{name}.pem"));
    let key = root.join(format!("{name}.key"));
    let output = Command::new("/usr/bin/openssl")
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
        output.status.success(),
        "test certificate generation failed"
    );
    (cert, key)
}
fn registry(root: &Path, cert: &Path, key: &Path) -> RegistryServer {
    let package = root.join("p14-tls-dep-0.1.0");
    fs::create_dir_all(package.join("src")).unwrap();
    fs::write(
        package.join("Cargo.toml"),
        "[package]\nname='p14-tls-dep'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::write(
        package.join("src/lib.rs"),
        "pub fn answer() -> u32 { 42 }\n",
    )
    .unwrap();
    let archive = Command::new("/usr/bin/tar")
        .args(["-czf", "-", "-C"])
        .arg(root)
        .arg("p14-tls-dep-0.1.0")
        .output()
        .unwrap();
    assert!(archive.status.success());
    let checksum = ag1_storage::digest(&archive.stdout);
    let identity =
        native_tls::Identity::from_pkcs8(&fs::read(cert).unwrap(), &fs::read(key).unwrap())
            .unwrap();
    let acceptor = native_tls::TlsAcceptor::new(identity).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let halt = stop.clone();
    let seen = requests.clone();
    let worker = thread::spawn(move || {
        while !halt.load(Ordering::Acquire) {
            let socket = match listener.accept() {
                Ok((socket, _)) => socket,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(e) => panic!("TLS listener: {e}"),
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let Ok(mut stream) = acceptor.accept(socket) else {
                seen.lock().unwrap().push("TLS rejected".into());
                continue;
            };
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                if stream.read(&mut byte).unwrap_or(0) == 0 {
                    break;
                }
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            // TLS 1.3 may let the server finish its handshake before the client
            // reports a certificate rejection and closes without sending HTTP.
            if request.is_empty() {
                seen.lock().unwrap().push("TLS closed without HTTP".into());
                continue;
            }
            let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
            seen.lock().unwrap().push(path.clone());
            let body = match path.as_str() {
                "/config.json" => json!({"dl":format!("https://127.0.0.1:{port}/crates/{{crate}}/{{version}}/download")}).to_string().into_bytes(),
                "/p1/4-/p14-tls-dep" => format!("{}\n", json!({"name":"p14-tls-dep","vers":"0.1.0","deps":[],"cksum":checksum,"features":{},"yanked":false})).into_bytes(),
                "/crates/p14-tls-dep/0.1.0/download" => archive.stdout.clone(),
                _ => Vec::new(),
            };
            let status = if body.is_empty() {
                "404 Not Found"
            } else {
                "200 OK"
            };
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.shutdown();
        }
    });
    RegistryServer {
        stop,
        requests,
        worker: Some(worker),
        port,
    }
}

#[test]
fn empty_cache_tls_fetch_and_untrusted_rejection() {
    let tmp = tempfile::tempdir().unwrap();
    let (cert, key) = certificate(tmp.path(), "trusted");
    let (wrong, _) = certificate(tmp.path(), "unrelated");
    let server = registry(tmp.path(), &cert, &key);
    for (mode, ca) in [("trusted", cert), ("untrusted", wrong)] {
        let case = tmp.path().join(mode);
        let ssl = case.join("ssl");
        fs::create_dir_all(&ssl).unwrap();
        // Exercise distro-style links whose target is outside /etc/ssl.
        symlink(&ca, ssl.join("cert.pem")).unwrap();
        let before = server.requests.lock().unwrap().len();
        let output = Command::new("/usr/bin/bwrap")
            .args([
                "--die-with-parent",
                "--unshare-user",
                "--unshare-pid",
                "--ro-bind",
                "/",
                "/",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--bind",
            ])
            .arg(tmp.path())
            .arg(tmp.path())
            .arg("--ro-bind")
            .arg(&ssl)
            .arg("/etc/ssl")
            .arg("--")
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tls::isolated_tls_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("AG1_P14_TLS_ROOT", &case)
            .env("AG1_P14_TLS_MODE", mode)
            .env("AG1_P14_TLS_PORT", server.port.to_string())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let seen = server.requests.lock().unwrap();
        let requests = &seen[before..];
        if mode == "trusted" {
            for path in [
                "/config.json",
                "/p1/4-/p14-tls-dep",
                "/crates/p14-tls-dep/0.1.0/download",
            ] {
                assert!(
                    requests.iter().any(|r| r == path),
                    "missing {path}: {requests:?}"
                );
            }
            assert_eq!(
                requests.iter().filter(|r| r.ends_with("/download")).count(),
                1
            );
            assert!(!requests.iter().any(|r| r == "TLS rejected"));
        } else {
            assert!(!requests.is_empty());
            assert!(
                requests
                    .iter()
                    .all(|r| r == "TLS rejected" || r == "TLS closed without HTTP"),
                "untrusted peer received HTTP: {requests:?}"
            );
        }
    }
}

// Invoked only in the parent's isolated mount namespace; ordinary suite runs
// leave this helper idle. Never override the real process's trust environment.
#[test]
#[ignore = "internal child worker; exercised by empty_cache_tls_fetch_and_untrusted_rejection"]
fn isolated_tls_worker() {
    let Some(root) = std::env::var_os("AG1_P14_TLS_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let trusted = std::env::var("AG1_P14_TLS_MODE").unwrap() == "trusted";
    let port: u16 = std::env::var("AG1_P14_TLS_PORT").unwrap().parse().unwrap();
    let project = root.join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir(project.join(".cargo")).unwrap();
    fs::write(project.join("Cargo.toml"), "[package]\nname='p14-tls-consumer'\nversion='0.1.0'\nedition='2024'\n[dependencies]\np14-tls-dep='=0.1.0'\n[workspace]\n").unwrap();
    fs::write(
        project.join("src/main.rs"),
        "fn main() { assert_eq!(p14_tls_dep::answer(), 42); }\n",
    )
    .unwrap();
    fs::write(project.join(".cargo/config.toml"), format!("[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\nregistry='sparse+https://127.0.0.1:{port}/'\n[net]\nretry=0\n[http]\ntimeout=5\nmultiplexing=false\n")).unwrap();
    assert!(!project.join("target/ag1-cargo").exists());
    let state = root.join("state");
    fs::create_dir(&state).unwrap();
    let cargo = PathBuf::from(env!("CARGO")).canonicalize().unwrap();
    let toolchain = cargo.parent().unwrap().parent().unwrap().to_owned();
    let workspaces = WorkspaceService::new().unwrap();
    let mut runtime = Runtime::load(
        workspaces.register(&project).unwrap(),
        workspaces.register(&state).unwrap(),
        env!("CARGO_BIN_EXE_ag1-process-host").as_ref(),
        Toolchains {
            node: None,
            npm: None,
            rustc: Some(cargo.with_file_name("rustc")),
            cargo: Some(cargo),
            godot: None,
            mounts: vec![toolchain],
        },
    )
    .unwrap();
    let target = runtime
        .project
        .targets
        .iter()
        .find(|t| t.kind == ag1_core::projects::Kind::Rust)
        .unwrap()
        .id
        .clone();
    for offline in if trusted {
        vec![false, true]
    } else {
        vec![false]
    } {
        let mut commands = runtime
            .command(ag1_core::projects::Kind::Rust, "check")
            .unwrap();
        if offline {
            commands[0].argv.push("--offline".into());
        }
        let cancel = AtomicBool::new(false);
        let control = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(20),
        };
        runtime
            .start(
                &target,
                "check",
                Launch {
                    commands,
                    readiness: None,
                    timeout_ms: 15_000,
                },
                &control,
            )
            .unwrap();
        let record = runtime.finish(&control).unwrap();
        if trusted {
            assert_eq!(
                record.exits,
                vec![Some(0)],
                "offline={offline}: {:?}",
                record.diagnostics
            );
            assert!(project.join("target/ag1-cargo/registry/cache").is_dir());
        } else {
            assert_eq!(record.exits, vec![Some(101)], "{:?}", record.diagnostics);
            let stderr = runtime.log(&record.id, 1, 0, 32768).unwrap();
            let text = String::from_utf8_lossy(&stderr);
            assert!(
                text.contains("SSL certificate problem")
                    || text.contains("certificate verify failed")
                    || text.contains("SSL certificate OpenSSL verify result"),
                "{text}"
            );
            assert!(!project.join("target/ag1-cargo/registry/cache").exists());
        }
    }
}
