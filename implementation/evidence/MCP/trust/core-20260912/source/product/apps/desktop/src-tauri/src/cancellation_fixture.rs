//! Diagnostic-build-only owned runtime. No model, shell, configurable executable,
//! tools, or external endpoint. A successful cancellation requires kill + wait
//! on the actual HTTP server Child; EOF and elapsed time never count as proof.
use ag1_core::{DomainError, PortFuture};
use ag1_platform::{
    client::Config,
    provider::http::{GenerationRuntime, HttpRequest},
};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn error(e: impl std::fmt::Display) -> DomainError {
    DomainError::Unavailable(e.to_string())
}
fn endpoint(origin: &str) -> Result<u16, String> {
    let port = origin
        .strip_prefix("http://127.0.0.1:")
        .ok_or("Fixture must use IPv4 loopback")?
        .parse::<u16>()
        .map_err(|_| "Invalid fixture port")?;
    if port == 0 {
        return Err("Fixture needs an explicit port".into());
    }
    Ok(port)
}
fn event(directory: &Path, value: Value) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("runtime-events.jsonl"))?;
    file.write_all(format!("{value}\n").as_bytes())
}
pub fn from_environment(config: &Config) -> Result<Option<Arc<dyn GenerationRuntime>>, String> {
    if std::env::var_os("AG1_P14_OWNED_FIXTURE").is_none() {
        return Ok(None);
    }
    if std::env::var("AG1_P14_OWNED_FIXTURE").as_deref() != Ok("1")
        || !config.fixture
        || config.runtime.is_some()
        || config.models.len() != 1
        || config.models[0]["provider"] != "fixture"
        || config.models[0]["id"] != "startup-fixture"
        || config.credential_env.is_some()
    {
        return Err("Owned fixture admission rejected".into());
    }
    let origin = config.models[0]["baseUrl"]
        .as_str()
        .ok_or("Fixture origin missing")?
        .to_owned();
    endpoint(&origin)?;
    let socket = PathBuf::from(
        std::env::var_os("AG1_VISUAL_SOCKET").ok_or("Fixture requires visual broker")?,
    );
    let directory = socket
        .parent()
        .ok_or("Fixture directory missing")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let scratch = tempfile::Builder::new()
        .prefix("owned-runtime-")
        .tempdir_in(&directory)
        .map_err(|e| e.to_string())?;
    Ok(Some(Arc::new(Runtime {
        origin,
        directory,
        scratch,
        child: Mutex::new(None),
    })))
}
struct Runtime {
    origin: String,
    directory: PathBuf,
    scratch: tempfile::TempDir,
    child: Mutex<Option<Child>>,
}
impl Runtime {
    fn prepare_owned(&self, request: &HttpRequest) -> Result<(), DomainError> {
        if request.url != format!("{}/chat/completions", self.origin) {
            return Err(DomainError::PermissionDenied);
        }
        let mut slot = self.child.lock().map_err(error)?;
        if let Some(child) = slot.as_mut() {
            return if child.try_wait().map_err(error)?.is_none() {
                Ok(())
            } else {
                Err(error("Fixture exited unexpectedly"))
            };
        }
        let ready = self.scratch.path().join("ready");
        if ready.exists() {
            fs::remove_file(&ready).map_err(error)?;
        }
        let child = Command::new(std::env::current_exe().map_err(error)?)
            .arg("--p14-cancellation-fixture")
            .arg(&self.origin)
            .arg(self.scratch.path())
            .arg(&self.directory)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(error)?;
        let pid = child.id();
        *slot = Some(child); // Retain custody even if readiness or receipt writing fails.
        event(&self.directory, json!({"event":"spawn", "pid":pid})).map_err(error)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if slot.as_mut().unwrap().try_wait().map_err(error)?.is_some() {
                return Err(error("Fixture startup failed"));
            }
            if fs::read_to_string(&ready).ok().as_deref() == Some(&pid.to_string()) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(error("Fixture readiness timed out"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn stop_owned(&self, cancellation: bool) -> Result<(), DomainError> {
        let mut slot = self.child.lock().map_err(error)?;
        let Some(child) = slot.as_mut() else {
            return if cancellation {
                Err(error("No owned generation to cancel"))
            } else {
                Ok(())
            };
        };
        if child.try_wait().map_err(error)?.is_some() {
            return Err(error("Fixture exited without cancellation proof"));
        }
        let pid = child.id();
        child.kill().map_err(error)?;
        let status = child.wait().map_err(error)?;
        *slot = None;
        event(
            &self.directory,
            json!({"event":if cancellation {"cancel_reaped"} else {"shutdown_reaped"},
            "pid":pid, "status":status.to_string(), "waited":true}),
        )
        .map_err(error)
    }
}
impl GenerationRuntime for Runtime {
    fn prepare<'a>(&'a self, request: &'a HttpRequest) -> PortFuture<'a, ()> {
        Box::pin(async move { self.prepare_owned(request) })
    }
    fn cancel_generation<'a>(&'a self, url: &'a str) -> PortFuture<'a, ()> {
        Box::pin(async move {
            if url != format!("{}/chat/completions", self.origin) {
                return Err(DomainError::PermissionDenied);
            }
            self.stop_owned(true)
        })
    }
    fn shutdown(&self) -> PortFuture<'_, ()> {
        Box::pin(async move { self.stop_owned(false) })
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Ok(slot) = self.child.get_mut() {
            if let Some(mut child) = slot.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
fn send(stream: &mut TcpStream, text: &str, finished: bool) -> std::io::Result<()> {
    write!(
        stream,
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content":text},"finish_reason":if finished {Some("stop")} else {None}}]})
    )?;
    if finished {
        stream.write_all(b"data: [DONE]\n\n")?;
    }
    stream.flush()
}
fn respond(mut stream: TcpStream, directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut header = String::new();
    reader.read_line(&mut header)?;
    if header != "POST /chat/completions HTTP/1.1\r\n" {
        return Err("Unexpected fixture request".into());
    }
    let mut length = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" {
            break;
        }
        if line.is_empty() || header.len() + line.len() > 65536 {
            return Err("Invalid headers".into());
        }
        header.push_str(&line);
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = length
        .filter(|n| *n <= 4 * 1024 * 1024)
        .ok_or("Missing/big body")?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let payload: Value = serde_json::from_slice(&body)?;
    if payload["model"] != "startup-fixture" || payload["stream"] != true {
        return Err("Unexpected model/mode".into());
    }
    let prompt = scripted_prompt(&payload)?;
    event(
        directory,
        json!({"event":"request", "pid":std::process::id(), "prompt":prompt}),
    )?;
    if prompt == "P14 OWNED before-output" {
        std::thread::sleep(Duration::from_secs(60)); // No headers or body; only owned-process cancellation can settle this.
        return Err("Before-output fixture was not cancelled".into());
    }
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
    )?;
    if prompt == "P14 OWNED recovery" {
        send(&mut stream, "OWNED_RECOVERY_COMPLETE", true)?;
        event(
            directory,
            json!({"event":"terminal", "pid":std::process::id()}),
        )?;
    } else {
        for _ in 0..400 {
            if send(&mut stream, "OWNED_STREAM_PULSE ", false).is_err() {
                // Disconnect does not stop the runtime: remain alive accepting
                // until the owner kills and reaps this process.
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        return Err("Streaming fixture was not cancelled".into());
    }
    Ok(())
}
fn scripted_prompt(payload: &Value) -> Result<String, &'static str> {
    // Runtime context can follow the human message using the user role too.
    // Select the latest exact scripted prompt, not the trailing context block.
    payload["messages"]
        .as_array()
        .ok_or("Missing messages")?
        .iter()
        .rev()
        .filter(|m| m["role"] == "user")
        .map(|user| {
            let content = &user["content"];
            content.as_str().map(str::to_owned).unwrap_or_else(|| {
                content
                    .as_array()
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter_map(|b| b["text"].as_str())
                            .collect::<String>()
                    })
                    .unwrap_or_default()
            })
        })
        .find(|prompt| {
            [
                "P14 OWNED streaming",
                "P14 OWNED before-output",
                "P14 OWNED recovery",
            ]
            .contains(&prompt.as_str())
        })
        .ok_or("Unexpected fixture prompt")
}
/// Early diagnostic binary entry: fixed loopback HTTP only, no GUI/model startup.
pub fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 5 {
        return Err("Invalid fixture arguments".into());
    }
    let port = endpoint(&args[2])?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    fs::write(
        Path::new(&args[3]).join("ready"),
        std::process::id().to_string(),
    )?;
    // Hard lifetime bound is failure-only; never accepted as cancellation proof.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        std::process::exit(70);
    });
    for stream in listener.incoming() {
        respond(stream?, Path::new(&args[4]))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn latest_scripted_prompt_ignores_trailing_runtime_context() {
        let payload = serde_json::json!({"messages":[
            {"role":"user","content":"P14 OWNED streaming"},
            {"role":"user","content":[{"type":"text","text":"P14 OWNED recovery"}]},
            {"role":"user","content":"Runtime context for this request"}
        ]});
        assert_eq!(
            super::scripted_prompt(&payload),
            Ok("P14 OWNED recovery".into())
        );
        assert!(super::scripted_prompt(&serde_json::json!({"messages":[{"role":"assistant","content":"P14 OWNED recovery"}]})).is_err());
    }
    #[test]
    fn endpoint_rejects_remote_and_ambiguous_addresses() {
        assert_eq!(super::endpoint("http://127.0.0.1:32123"), Ok(32123));
        for value in [
            "http://localhost:32123",
            "https://127.0.0.1:32123",
            "http://127.0.0.1:0",
            "http://127.0.0.1:32123/",
            "http://example.com:32123",
        ] {
            assert!(super::endpoint(value).is_err(), "{value}");
        }
    }
}
