//! Pinned Godot v1 JSONL. Native host remains transaction and runtime owner.
use crate::{io_error, resource_fs::ScanControl, workspace::Workspace};
use ag1_core::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};
pub const SCHEMA_SHA256: &str = "14cccf01ec0063edf774b18ec14c7094f373816ffb23adc924e9f86c669dafaf";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub protocol_version: u32,
    pub kind: String,
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_nonce: Option<String>,
}
fn invalid(s: &str) -> DomainError {
    DomainError::InvalidInput(s.into())
}
fn token(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}
pub fn decode(line: &[u8]) -> Result<Envelope, DomainError> {
    if line.len() > 1048576 {
        return Err(DomainError::LimitExceeded("Godot control line".into()));
    }
    if line.contains(&b'\n') || line.contains(&b'\r') {
        return Err(invalid("one Godot JSONL record required"));
    }
    let e: Envelope = serde_json::from_slice(line).map_err(|_| invalid("Godot envelope"))?;
    if e.protocol_version != 1 {
        return Err(DomainError::Unavailable(
            "Godot protocol version mismatch".into(),
        ));
    }
    if ![
        "host.hello",
        "sidecar.hello",
        "host.request",
        "host.response",
        "sidecar.request",
        "sidecar.response",
        "event",
        "error",
    ]
    .contains(&e.kind.as_str())
        || !e.payload.is_object()
        || e.correlation_id.as_ref().is_some_and(|v| !token(v, 1, 128))
        || e.session_nonce.as_ref().is_some_and(|v| !token(v, 16, 128))
    {
        return Err(invalid("Godot envelope fields"));
    }
    fn depth(v: &Value, n: usize) -> bool {
        n <= 32
            && match v {
                Value::Array(a) => a.iter().all(|v| depth(v, n + 1)),
                Value::Object(o) => o.values().all(|v| depth(v, n + 1)),
                _ => true,
            }
    }
    if !depth(&e.payload, 0) {
        return Err(DomainError::LimitExceeded("Godot JSON depth".into()));
    }
    Ok(e)
}
pub fn encode(e: &Envelope) -> Result<Vec<u8>, DomainError> {
    let mut b = serde_json::to_vec(e).map_err(|_| invalid("Godot encoding"))?;
    decode(&b)?;
    b.push(b'\n');
    Ok(b)
}
#[derive(Clone, Debug)]
pub struct Handshake {
    pub nonce: String,
    pub engine_build: String,
    pub engine_commit: String,
    pub project_path: String,
}
pub fn handshake(e: &Envelope, workspace: &Workspace) -> Result<Handshake, DomainError> {
    workspace.verify_root()?;
    if e.kind != "host.hello"
        || e.correlation_id.is_none()
        || e.session_nonce.is_none()
        || e.payload["schema_sha256"] != SCHEMA_SHA256
    {
        return Err(DomainError::PermissionDenied);
    }
    let p = &e.payload;
    let project_path = p["project_path"]
        .as_str()
        .ok_or_else(|| invalid("Godot project path"))?;
    if !Path::new(project_path).is_absolute()
        || Path::new(project_path).canonicalize().map_err(io_error)? != workspace.root()
    {
        return Err(DomainError::PermissionDenied);
    }
    let capabilities = p["capabilities"]
        .as_array()
        .ok_or_else(|| invalid("Godot capabilities"))?;
    if !["host.request.v1", "sidecar.request.v1"]
        .iter()
        .all(|c| capabilities.contains(&json!(c)))
    {
        return Err(DomainError::Unavailable(
            "Godot host requires bidirectional v1 requests".into(),
        ));
    }
    Ok(Handshake {
        nonce: e.session_nonce.clone().unwrap(),
        engine_build: p["engine_build"]
            .as_str()
            .ok_or_else(|| invalid("Godot engine build"))?
            .into(),
        engine_commit: p["engine_commit"]
            .as_str()
            .ok_or_else(|| invalid("Godot engine commit"))?
            .into(),
        project_path: project_path.into(),
    })
}
pub fn allowed(method: &str) -> bool {
    [
        "godot.describe",
        "godot.scene.get_state",
        "godot.selection.get_state",
        "godot.scene.resolve_node",
        "godot.project.get_state",
        "godot.project.list_resources",
        "godot.project.resolve_resource",
        "godot.observations.query",
        "godot.project.dependency_index",
        "godot.code.languages",
        "godot.code.inspect",
        "godot.capabilities.get_state",
        "godot.visual.capture",
        "godot.run.capture",
        "godot.code.build",
        "godot.code.build.status",
        "godot.file.write.text",
        "godot.file.write.patch",
        "godot.resource.mutate.create",
    ]
    .contains(&method)
        || method
            .strip_prefix("godot.transaction.")
            .is_some_and(|v| ["begin", "status", "list", "commit", "rollback"].contains(&v))
        || method.strip_prefix("godot.scene.mutate.").is_some_and(|v| {
            [
                "create_node",
                "create_scene",
                "set_property",
                "assign_resource",
                "instantiate_scene",
                "reparent_node",
                "remove_node",
                "save",
            ]
            .contains(&v)
        })
        || method.strip_prefix("godot.run.").is_some_and(|v| {
            [
                "start",
                "status",
                "wait_debugger",
                "tree",
                "inspect",
                "input",
                "wait_frames",
                "wait_condition",
                "assert",
                "performance",
                "stop",
            ]
            .contains(&v)
        })
}
fn mutating(method: &str) -> bool {
    method.contains(".mutate.")
        || method.contains(".write.")
        || [
            "godot.transaction.begin",
            "godot.transaction.commit",
            "godot.transaction.rollback",
            "godot.code.build",
            "godot.run.start",
            "godot.run.input",
            "godot.run.wait_frames",
            "godot.run.wait_condition",
            "godot.run.assert",
            "godot.run.stop",
        ]
        .contains(&method)
}
/// One request at a time is below the pinned 256-pending bound. EOF/watchdog
/// failure closes the broker; uncertain mutations are never implicitly retried.
pub struct Broker {
    stream: UnixStream,
    pub identity: Handshake,
    next: u64,
    closed: bool,
    transactions: BTreeSet<String>,
    buffer: Vec<u8>,
}
impl Broker {
    pub fn accept(
        mut stream: UnixStream,
        workspace: &Workspace,
        control: &ScanControl<'_>,
    ) -> Result<Self, DomainError> {
        stream
            .set_read_timeout(Some(Duration::from_millis(20)))
            .map_err(io_error)?;
        stream
            .set_write_timeout(Some(Duration::from_millis(20)))
            .map_err(io_error)?;
        let mut buffer = vec![];
        let hello = read_line(&mut stream, &mut buffer, control)?;
        let identity = handshake(&hello, workspace)?;
        let response = Envelope {
            protocol_version: 1,
            kind: "sidecar.hello".into(),
            correlation_id: hello.correlation_id,
            session_nonce: Some(identity.nonce.clone()),
            payload: json!({"schema_sha256":SCHEMA_SHA256,"sidecar_build":"ag1-rust-p11","sidecar_source_commit":"native","capabilities":["host.request.v1","sidecar.request.v1"]}),
        };
        stream.write_all(&encode(&response)?).map_err(io_error)?;
        Ok(Self {
            stream,
            identity,
            next: 0,
            closed: false,
            transactions: BTreeSet::new(),
            buffer,
        })
    }
    /// Build-mode authorization is host-supplied after the normal policy fence.
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        build_mode: bool,
        control: &ScanControl<'_>,
    ) -> Result<Value, DomainError> {
        control.check()?;
        if self.closed {
            return Err(DomainError::Unavailable(
                "Godot host disconnected; reconnect and inspect transaction status".into(),
            ));
        }
        if !allowed(method) || !params.is_object() || (!build_mode && mutating(method)) {
            return Err(DomainError::PermissionDenied);
        }
        if method.contains(".mutate.") || method.contains(".write.") || method == "godot.code.build"
        {
            let tx = params["transaction_id"]
                .as_str()
                .ok_or_else(|| invalid("Godot transaction required"))?;
            if !self.transactions.contains(tx) {
                return Err(DomainError::Conflict);
            }
            if method != "godot.code.build"
                && params["operation_id"]
                    .as_str()
                    .is_none_or(|s| !token(s, 1, 128))
            {
                return Err(invalid("stable Godot operation ID required"));
            }
        }
        self.next += 1;
        let correlation = format!("host-rust-{}", self.next);
        let e = Envelope {
            protocol_version: 1,
            kind: "host.request".into(),
            correlation_id: Some(correlation.clone()),
            session_nonce: Some(self.identity.nonce.clone()),
            payload: json!({"method":method,"params":params}),
        };
        let outcome = (|| {
            self.stream.write_all(&encode(&e)?).map_err(io_error)?;
            let response = read_line(&mut self.stream, &mut self.buffer, control)?;
            if response.session_nonce.as_deref() != Some(&self.identity.nonce)
                || response.correlation_id.as_deref() != Some(&correlation)
            {
                return Err(DomainError::PermissionDenied);
            }
            if response.kind == "error" || response.payload["ok"] != true {
                return Err(DomainError::Unavailable(format!(
                    "Godot host error: {}",
                    response.payload["code"].as_str().unwrap_or("host_error")
                )));
            }
            if response.kind != "host.response" {
                return Err(invalid("Godot response kind"));
            }
            Ok(response.payload["result"].clone())
        })();
        match &outcome {
            Ok(result) => {
                if ["godot.transaction.begin", "godot.transaction.status"].contains(&method)
                    && let Some(tx) = result["transaction_id"]
                        .as_str()
                        .or_else(|| params["transaction_id"].as_str())
                {
                    self.transactions.insert(tx.into());
                }
                if ["godot.transaction.commit", "godot.transaction.rollback"].contains(&method)
                    && let Some(tx) = params["transaction_id"].as_str()
                {
                    self.transactions.remove(tx);
                }
            }
            Err(_) => self.close(),
        }
        outcome
    }
    pub fn close(&mut self) {
        self.closed = true;
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
fn read_line(
    stream: &mut UnixStream,
    buffer: &mut Vec<u8>,
    control: &ScanControl<'_>,
) -> Result<Envelope, DomainError> {
    loop {
        control.check()?;
        if let Some(n) = buffer.iter().position(|b| *b == b'\n') {
            let line = buffer.drain(..=n).collect::<Vec<_>>();
            return decode(&line[..n]);
        }
        let mut bytes = [0; 8192];
        match stream.read(&mut bytes) {
            Ok(0) => {
                return Err(DomainError::Unavailable(
                    "Godot parent watchdog: EOF".into(),
                ));
            }
            Ok(n) => buffer.extend_from_slice(&bytes[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(io_error(e)),
        }
        if buffer.len() > 1048576 {
            return Err(DomainError::LimitExceeded("Godot control buffer".into()));
        }
    }
}
/// Exact private render artifact; callers retain bytes through the P05 branch ACL.
/// Vision admission is explicit and must come from the selected model/projector.
pub fn capture(
    workspace: &Workspace,
    result: &Value,
    vision: bool,
) -> Result<Vec<u8>, DomainError> {
    if !vision {
        return Err(DomainError::Unavailable(
            "Selected model/projector has no vision capability".into(),
        ));
    }
    let hash = result["sha256"]
        .as_str()
        .filter(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(|| invalid("Godot image hash"))?;
    let path = format!(".godot/ai/visual/blobs/{hash}.png");
    if result["private_path"] != format!("res://{path}") {
        return Err(DomainError::PermissionDenied);
    }
    let snapshot = workspace.snapshot(&path, 8 * 1024 * 1024)?;
    if snapshot.revision.sha256 != hash
        || result["bytes"].as_u64() != Some(snapshot.bytes.len() as u64)
    {
        return Err(DomainError::Conflict);
    }
    let decoded = image::load_from_memory_with_format(&snapshot.bytes, image::ImageFormat::Png)
        .map_err(|_| invalid("Godot PNG"))?;
    if decoded.width() > 2048
        || decoded.height() > 2048
        || result["width"].as_u64() != Some(decoded.width() as u64)
        || result["height"].as_u64() != Some(decoded.height() as u64)
    {
        return Err(invalid("Godot image dimensions"));
    }
    Ok(snapshot.bytes)
}
/// Verify the existing backend-stager format without launching/rebuilding/tuning.
pub fn verify_stage(directory: &Workspace, manifest: &Value) -> Result<(), DomainError> {
    if manifest["formatVersion"] != 1 {
        return Err(invalid("Godot backend stage version"));
    }
    let files = manifest["files"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 1000)
        .ok_or_else(|| invalid("Godot stage files"))?;
    let mut seen = BTreeSet::new();
    for file in files {
        let path = file["path"].as_str().ok_or_else(|| invalid("stage path"))?;
        if !seen.insert(path) {
            return Err(invalid("duplicate staged path"));
        }
        if file["kind"] == "file" {
            let revision =
                directory.stream_runtime_material(path, 2 * 1024 * 1024 * 1024, |_| Ok(()))?;
            if file["sha256"] != revision.sha256
                || file["size"].as_u64() != Some(revision.size)
                || file["mode"].as_u64() != Some(u64::from(revision.mode))
            {
                return Err(DomainError::Conflict);
            }
        } else if file["kind"] == "symlink" {
            let (target, revision) = directory.link_snapshot(path)?;
            if Path::new(&target).components().count() != 1
                || !matches!(
                    Path::new(&target).components().next(),
                    Some(std::path::Component::Normal(_))
                )
                || file["target"] != target
                || file["mode"].as_u64() != Some(u64::from(revision.mode))
            {
                return Err(DomainError::Conflict);
            }
            let resolved = Path::new(path)
                .parent()
                .unwrap_or(Path::new(""))
                .join(&target)
                .to_string_lossy()
                .into_owned();
            if !files.iter().any(|f| f["path"] == resolved) {
                return Err(DomainError::PermissionDenied);
            }
        } else {
            return Err(invalid("staged file kind"));
        }
    }
    let server = manifest["serverPath"]
        .as_str()
        .ok_or_else(|| invalid("staged server path"))?;
    let revision = directory.stream_runtime_material(server, 2 * 1024 * 1024 * 1024, |_| Ok(()))?;
    if manifest["serverSha256"] != revision.sha256 {
        return Err(DomainError::Conflict);
    }
    Ok(())
}
/// Copies an already admitted preserved stage into exactly one new private
/// destination. No source discovery, download, engine rebuild or tuning occurs.
pub fn copy_stage(
    source: &Workspace,
    parent: &std::sync::Arc<Workspace>,
    name: &str,
    manifest: &Value,
    control: &ScanControl<'_>,
) -> Result<std::sync::Arc<Workspace>, DomainError> {
    verify_stage(source, manifest)?;
    control.check()?;
    parent.create_directory_document_checked(
        name,
        "backend-manifest.json",
        &serde_json::to_vec_pretty(manifest).map_err(|_| invalid("stage manifest"))?,
        || control.check(),
    )?;
    let pin = parent.pin_directory(name)?;
    let result = (|| {
        let destination =
            crate::workspace::WorkspaceService::new()?.register(&parent.root().join(name))?;
        for file in manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["kind"] == "file")
        {
            control.check()?;
            let path = file["path"].as_str().unwrap();
            destination.copy_runtime_material(source, path, control)?;
        }
        for file in manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["kind"] == "symlink")
        {
            control.check()?;
            destination.create_local_link(
                file["path"].as_str().unwrap(),
                file["target"].as_str().unwrap(),
            )?;
        }
        verify_stage(source, manifest)?;
        verify_stage(&destination, manifest)?;
        Ok(destination)
    })();
    if result.is_err() {
        parent.remove_directory_tree_checked(pin, || Ok(()))?;
    }
    result
}
