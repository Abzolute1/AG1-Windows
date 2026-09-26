//! Controlled test child. Never shipped as a runtime adapter or invoked with user data.
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::Duration,
};
#[allow(clippy::zombie_processes)] // The orphan scenario deliberately transfers reaping to the private helper.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(target_os = "linux")]
    if args.len() == 2 && args[1].parse::<i32>().is_ok() {
        use std::os::{fd::FromRawFd, unix::net::UnixStream};
        // Explicit test-only helper: expose a real authenticated process whose
        // settlement outlasts the generic caller's three-second wait budget.
        let mut control = unsafe { UnixStream::from_raw_fd(args[1].parse().unwrap()) };
        let mut length = [0; 4];
        control.read_exact(&mut length).unwrap();
        let length = u32::from_le_bytes(length) as usize;
        assert!(length <= 1024 * 1024);
        let mut bytes = vec![0; length];
        control.read_exact(&mut bytes).unwrap();
        let spec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(spec["argv"], serde_json::json!(["settlement-delay"]));
        control.write_all(&[1]).unwrap();
        assert_eq!(control.read(&mut [0]).unwrap(), 0);
        std::thread::sleep(Duration::from_secs(4));
        return;
    }
    match args.get(1).map(String::as_str).unwrap_or("") {
        "package" => package_fixture(&args[2..]),
        "--model" => {
            // P06 fake model server: records the exact launch and waits for its owner.
            // This executable performs no inference and never opens a model endpoint.
            let record = std::env::var("AG1_MODEL_FIXTURE_RECORD").unwrap();
            std::fs::write(record,serde_json::to_vec(&serde_json::json!({"pid":std::process::id(),"args":&args[1..],"library":std::env::var("LD_LIBRARY_PATH").ok()})).unwrap()).unwrap();
            loop {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        "echo" => {
            println!("cwd={}", std::env::current_dir().unwrap().display());
            println!(
                "token={}",
                std::env::var("FIXTURE_TOKEN").unwrap_or_default()
            );
            println!("ambient={}", std::env::var("AG1_FIXTURE_AMBIENT").is_ok());
        }
        "input" => {
            let mut input = [0; 32];
            let count = std::io::stdin().read(&mut input).unwrap();
            std::io::stdout().write_all(&input[..count]).unwrap();
        }
        "protocol" => {
            eprintln!("diagnostic only");
            println!("{{\"type\":\"ready\"}}");
        }
        "address-space" => {
            #[cfg(target_os = "linux")]
            unsafe {
                let mut limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                assert_eq!(libc::getrlimit(libc::RLIMIT_AS, &mut limit), 0);
                println!("{} {}", limit.rlim_cur, limit.rlim_max);
            }
        }
        "deny-mutate" => {
            #[cfg(target_os = "linux")]
            {
                let service = ag1_platform::workspace::WorkspaceService::new().unwrap();
                let workspace = service.register(std::path::Path::new(&args[2])).unwrap();
                let snapshot = workspace.snapshot("file", 100).unwrap();
                if unsafe { libc::geteuid() } == 0 {
                    unsafe {
                        libc::setgid(65534);
                        libc::setuid(65534);
                    }
                }
                assert!(matches!(
                    workspace.replace("file", &snapshot.revision, b"bad"),
                    Err(ag1_core::DomainError::PermissionDenied)
                ));
            }
        }
        "flood" => {
            let mut stdout = std::io::stdout().lock();
            let bytes = [b'x'; 8192];
            for _ in 0..1280 {
                stdout.write_all(&bytes).unwrap();
            }
            stdout.write_all(b"TAIL-END").unwrap();
        }
        "hold" => {
            #[cfg(target_os = "linux")]
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
            }
            std::fs::write(&args[2], std::process::id().to_string()).unwrap();
            loop {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        "descendant" | "orphan" => {
            let executable = std::env::current_exe().unwrap();
            let mut child = Command::new(executable)
                .args(["hold", &args[2]])
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            if args[1] == "descendant" {
                let _ = child.wait();
            } else {
                for _ in 0..200 {
                    if std::path::Path::new(&args[2]).is_file() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        "deny-read" => {
            #[cfg(target_os = "linux")]
            {
                // Run only in a dedicated fixture child; does not change the test runner's identity.
                if unsafe { libc::geteuid() } == 0 {
                    unsafe {
                        libc::setgid(65534);
                        libc::setuid(65534);
                    }
                }
                assert!(std::fs::read(&args[2]).is_err());
            }
        }
        _ => std::process::exit(2),
    }
}

/// Deterministic package command port: only modifies the test's explicitly
/// supplied temporary root. It never invokes npm/git or uses the network.
fn package_fixture(args: &[String]) {
    use std::fs::{self, OpenOptions};
    let root = std::env::var("AG1_PACKAGE_FIXTURE_ROOT").unwrap();
    let root = std::path::Path::new(&root);
    let marker = root.join("commands.jsonl");
    let mut log = OpenOptions::new()
        .append(true)
        .create(true)
        .open(marker)
        .unwrap();
    writeln!(log, "{}", serde_json::json!({"args":args,"cwd":std::env::current_dir().unwrap(),"prefix":std::env::var("npm_config_prefix").ok(),"userconfig":std::env::var("npm_config_userconfig").ok(),"home":std::env::var("HOME").ok()})).unwrap();
    log.sync_all().unwrap();
    eprintln!("fixture diagnostic");
    if args.iter().any(|a| a == "slow") {
        fs::write(root.join("started"), std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(30));
    }
    if args.iter().any(|a| a == "fail") {
        println!("fixture failure output");
        std::process::exit(17);
    }
    if args.iter().any(|a| a == "flood") {
        let mut stdout = std::io::stdout().lock();
        for _ in 0..1024 {
            stdout.write_all(&[b'x'; 8192]).unwrap();
        }
        return;
    }
    let command = args.get(1).map(String::as_str).unwrap_or("");
    let bound = |path: &std::path::Path| {
        assert!(path.starts_with(root));
        assert!(
            !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        );
    };
    if args[0] == "git-query" {
        if command == "rev-parse" {
            println!(
                "{}",
                if args.iter().any(|a| a == "--abbrev-ref") {
                    "origin/main".into()
                } else {
                    "a".repeat(40)
                }
            );
        } else if command == "ls-remote" {
            println!("{}\t{}", "b".repeat(40), args.last().unwrap());
        } else {
            panic!("unexpected fixture Git query");
        }
    } else if args[0] == "npm" {
        let prefix = args
            .windows(2)
            .find(|p| p[0] == "--prefix")
            .map(|p| p[1].clone())
            .unwrap_or_else(|| std::env::var("npm_config_prefix").unwrap());
        let prefix = std::path::Path::new(&prefix);
        bound(prefix);
        if command == "root" {
            println!("{}/lib/node_modules", prefix.display());
            return;
        }
        let modules = prefix.join(if args.iter().any(|a| a == "-g") {
            "lib/node_modules"
        } else {
            "node_modules"
        });
        if command == "uninstall" {
            if modules.join("fixture").exists() {
                fs::remove_dir_all(modules.join("fixture")).unwrap();
            }
        } else {
            fs::create_dir_all(modules.join("fixture")).unwrap();
            let version = args
                .get(2)
                .and_then(|spec| spec.strip_prefix("fixture@"))
                .filter(|version| *version != "latest")
                .unwrap_or("1.0.0");
            fs::write(
                modules.join("fixture/package.json"),
                serde_json::json!({"version":version}).to_string(),
            )
            .unwrap();
        }
        println!("fixture npm complete");
    } else if command == "clone" {
        let target = std::path::Path::new(args.last().unwrap());
        bound(target);
        fs::create_dir_all(target).unwrap();
        fs::write(target.join("package.json"), b"{\"name\":\"fixture\"}").unwrap();
        println!("fixture clone complete");
    } else if command == "rev-parse" {
        println!(
            "{}",
            if args.iter().any(|a| a == "--abbrev-ref") {
                "origin/main"
            } else {
                "same-head"
            }
        );
    } else {
        println!("fixture git complete");
    }
}
