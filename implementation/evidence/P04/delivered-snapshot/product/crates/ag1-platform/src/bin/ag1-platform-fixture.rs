//! Controlled test child. Never shipped as a runtime adapter or invoked with user data.
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::Duration,
};
#[allow(clippy::zombie_processes)] // The orphan scenario deliberately transfers reaping to the private helper.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str).unwrap_or("") {
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
