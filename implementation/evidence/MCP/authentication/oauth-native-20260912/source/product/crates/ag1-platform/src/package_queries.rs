//! Package query composition over registered resource roots, native web transport
//! and a separately granted Git port. No registry endpoint or credentials come
//! from package manifests or extension arguments.
use crate::{
    provider::http::cancellation,
    resource_fs::{ResourceFs, ScanControl},
    web_http::{WebHttp, WebPurpose, WebResolver, WebResponse},
};
use ag1_core::{
    DomainError, PortFuture,
    resources::{
        package_queries::{self, Port},
        packages,
    },
};
use serde_json::{Value, json};
use std::{path::Path, sync::atomic::AtomicBool, time::Instant};

/// An embedding must store original bytes before acknowledging this call. The
/// operation/session ACL belongs to the embedding, never to the remote response.
pub trait Evidence: Sync {
    fn capture(&self, kind: &str, metadata: Value, bytes: &[u8]) -> Result<(), DomainError>;
}
impl<E: Evidence + ?Sized> Evidence for &E {
    fn capture(&self, kind: &str, metadata: Value, bytes: &[u8]) -> Result<(), DomainError> {
        (**self).capture(kind, metadata, bytes)
    }
}
/// P05 evidence adapter. The caller supplies a session/operation-bound store;
/// records retain binary bytes in a versioned envelope with native provenance.
pub struct StoredEvidence<A> {
    state: std::sync::Mutex<EvidenceState<A>>,
}
struct EvidenceState<A> {
    port: A,
    artifacts: Vec<ag1_core::context::ContextArtifact>,
    bytes: usize,
    failed: bool,
}
impl<A: ag1_core::context::ContextArtifactPort + Send> StoredEvidence<A> {
    pub fn new(port: A) -> Self {
        Self {
            state: std::sync::Mutex::new(EvidenceState {
                port,
                artifacts: Vec::new(),
                bytes: 0,
                failed: false,
            }),
        }
    }
    pub fn artifacts(&self) -> Result<Vec<ag1_core::context::ContextArtifact>, DomainError> {
        let state = self
            .state
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        if state.failed {
            return Err(DomainError::DurabilityUnknown);
        }
        Ok(state.artifacts.clone())
    }
}
impl<A: ag1_core::context::ContextArtifactPort + Send> Evidence for StoredEvidence<A> {
    fn capture(&self, kind: &str, metadata: Value, bytes: &[u8]) -> Result<(), DomainError> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        if ![
            "package_manifest",
            "package_registry",
            "package_git_stdout",
            "package_git_stderr",
        ]
        .contains(&kind)
            || bytes.len() > 1024 * 1024
        {
            return Err(DomainError::LimitExceeded(
                "package query evidence input".into(),
            ));
        }
        packages::validate_settings_document(&metadata)?;
        if metadata.to_string().len() > 65536 {
            return Err(DomainError::LimitExceeded(
                "package query evidence metadata".into(),
            ));
        }
        let envelope = json!({"version":1,"kind":kind,"metadata":metadata,"bytesBase64":STANDARD.encode(bytes)}).to_string();
        let mut state = self
            .state
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        if state.failed {
            return Err(DomainError::DurabilityUnknown);
        }
        if state.artifacts.len() >= 8192
            || envelope.len() > (64 * 1024 * 1024usize).saturating_sub(state.bytes)
        {
            return Err(DomainError::LimitExceeded("package query evidence".into()));
        }
        let artifact = match state.port.put_evidence(
            &envelope,
            "package-query-original",
            &json!({"type":"system","name":"packageUpdates"}),
            &[],
            None,
        ) {
            Ok(artifact) => artifact,
            Err(_) => {
                state.failed = true;
                return Err(DomainError::DurabilityUnknown);
            }
        };
        state.bytes += envelope.len();
        state.artifacts.push(artifact);
        Ok(())
    }
}
/// Host-granted Git access to owned installations, including any configured
/// remote helpers. Implementations must capture stdout/stderr, bind the command
/// to its cancellation lifetime, and never accept arbitrary commands here.
pub trait GitQueries: Sync {
    fn capture<'a>(
        &'a self,
        installed: &'a str,
        args: Vec<String>,
        remote: bool,
        deadline: Instant,
        cancel: &'a AtomicBool,
    ) -> PortFuture<'a, String>;
}
pub struct QueryPort<'a, R, G, E> {
    fs: &'a ResourceFs,
    http: &'a WebHttp<R>,
    git: &'a G,
    evidence: &'a E,
    registry: reqwest::Url,
    control: ScanControl<'a>,
    slots: tokio::sync::Semaphore,
}
impl<'a, R: WebResolver, G: GitQueries, E: Evidence> QueryPort<'a, R, G, E> {
    /// registry_origin is a native host setting (normally
    /// https://registry.npmjs.org); exact loopback origins are useful in fixtures.
    pub fn new(
        fs: &'a ResourceFs,
        http: &'a WebHttp<R>,
        git: &'a G,
        evidence: &'a E,
        registry_origin: &str,
        control: ScanControl<'a>,
    ) -> Result<Self, DomainError> {
        let registry = reqwest::Url::parse(registry_origin)
            .map_err(|_| DomainError::InvalidInput("package registry origin".into()))?;
        if registry_origin.len() > 8192
            || !matches!(registry.scheme(), "http" | "https")
            || registry.host_str().is_none()
            || !registry.username().is_empty()
            || registry.password().is_some()
            || registry.path() != "/"
            || registry.query().is_some()
            || registry.fragment().is_some()
        {
            return Err(DomainError::InvalidInput("package registry origin".into()));
        }
        control.check()?;
        Ok(Self {
            fs,
            http,
            git,
            evidence,
            registry,
            control,
            slots: tokio::sync::Semaphore::new(package_queries::CONCURRENCY),
        })
    }
    async fn wait_cancelled(&self) -> DomainError {
        loop {
            if let Err(error) = self.control.check() {
                return error;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
    fn capture_http(&self, response: &WebResponse) -> Result<(), DomainError> {
        self.evidence.capture("package_registry", json!({"requestedUrl":response.requested_url,
            "finalUrl":response.final_url,"redirectChain":response.redirect_chain,"status":response.status,
            "contentType":response.content_type}), &response.bytes)
    }
}
impl<R: WebResolver, G: GitQueries, E: Evidence> Port for QueryPort<'_, R, G, E> {
    fn checkpoint(&self) -> Result<(), DomainError> {
        self.control.check()
    }
    fn exists<'a>(&'a self, installed: &'a str) -> PortFuture<'a, bool> {
        Box::pin(async move {
            self.control.check()?;
            self.fs.exists(Path::new(installed))
        })
    }
    fn manifest<'a>(&'a self, installed: &'a str) -> PortFuture<'a, Option<Value>> {
        Box::pin(async move {
            self.control.check()?;
            let path = Path::new(installed).join("package.json");
            let (bytes, revision, canonical) = match self.fs.read(&path, 1024 * 1024, &self.control)
            {
                Ok(value) => value,
                Err(DomainError::NotFound) => return Ok(None),
                Err(error) => return Err(error),
            };
            self.evidence
                .capture(
                    "package_manifest",
                    json!({"path":path,"canonicalPath":canonical,"sha256":revision.sha256}),
                    &bytes,
                )
                .map_err(|_| DomainError::DurabilityUnknown)?;
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| DomainError::InvalidInput("package manifest JSON".into()))?;
            packages::validate_settings_document(&value)
                .map_err(|_| DomainError::LimitExceeded("package manifest JSON".into()))?;
            self.control.check()?;
            Ok(Some(value))
        })
    }
    fn latest_npm<'a>(&'a self, name: &'a str, timeout_ms: u64) -> PortFuture<'a, Option<Value>> {
        Box::pin(async move {
            self.control.check()?;
            // Require the already parsed package name, so a spec cannot smuggle a
            // query string, fragment or a second origin into a registry request.
            let source = packages::parse(&format!("npm:{name}"))?;
            if !matches!(source, packages::Source::Npm { name: ref parsed, pinned: false, .. } if parsed == name)
                || name.contains(['?', '#', '%', '\\', ':'])
            {
                return Err(DomainError::InvalidInput("registry package name".into()));
            }
            let _slot = tokio::select! {
                error = self.wait_cancelled() => return Err(error),
                slot = self.slots.acquire() => slot.map_err(|_| DomainError::Unavailable("package query slots".into()))?,
            };
            let mut url = self.registry.clone();
            url.set_path(&format!("/{name}/latest"));
            let deadline = self.control.deadline.min(
                Instant::now()
                    + std::time::Duration::from_millis(timeout_ms.min(package_queries::TIMEOUT_MS)),
            );
            let (_owner, mut cancel) = cancellation();
            let response = tokio::select! {
                error = self.wait_cancelled() => return Err(error),
                response = self.http.fetch_for(url.as_str(), None, WebPurpose::SearchJson, tokio::time::Instant::from_std(deadline), &mut cancel) => response,
            };
            let response = match response {
                // A network timeout is an ordinary failed update check in the
                // SDK. An expired owner deadline remains explicit cancellation.
                Err(DomainError::TimedOut) if self.control.check().is_ok() => {
                    return Err(DomainError::Unavailable(
                        "package registry timed out".into(),
                    ));
                }
                value => value?,
            };
            self.capture_http(&response)
                .map_err(|_| DomainError::DurabilityUnknown)?;
            self.control.check()?;
            if !(200..300).contains(&response.status) {
                return Err(DomainError::Unavailable(format!(
                    "Failed to fetch npm registry: {}",
                    response.status
                )));
            }
            let document: Value = serde_json::from_slice(&response.bytes)
                .map_err(|_| DomainError::InvalidInput("package registry JSON".into()))?;
            packages::validate_settings_document(&document)
                .map_err(|_| DomainError::LimitExceeded("package registry JSON".into()))?;
            Ok(document.get("version").cloned())
        })
    }
    fn git<'a>(
        &'a self,
        installed: &'a str,
        args: Vec<String>,
        remote: bool,
        timeout_ms: u64,
    ) -> PortFuture<'a, String> {
        Box::pin(async move {
            self.control.check()?;
            if !self.fs.admits_path(Path::new(installed)) {
                return Err(DomainError::PermissionDenied);
            }
            let allowed = if remote {
                args.len() == 3
                    && args[0] == "ls-remote"
                    && args[1] == "origin"
                    && (args[2] == "HEAD" || args[2].starts_with("refs/heads/"))
            } else {
                args == ["rev-parse", "HEAD"]
                    || args == ["rev-parse", "--abbrev-ref", "@{upstream}"]
            };
            if !allowed
                || args
                    .iter()
                    .any(|a| a.len() > 8192 || a.contains(['\0', '\n', '\r']))
            {
                return Err(DomainError::PermissionDenied);
            }
            let _slot = tokio::select! {
                error = self.wait_cancelled() => return Err(error),
                slot = self.slots.acquire() => slot.map_err(|_| DomainError::Unavailable("package query slots".into()))?,
            };
            let deadline = self.control.deadline.min(
                Instant::now()
                    + std::time::Duration::from_millis(timeout_ms.min(package_queries::TIMEOUT_MS)),
            );
            let value = tokio::select! {
                error = self.wait_cancelled() => return Err(error),
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err(DomainError::TimedOut),
                value = self.git.capture(installed, args, remote, deadline, self.control.cancel) => value,
            };
            if matches!(value, Err(DomainError::TimedOut)) && self.control.check().is_ok() {
                return Err(DomainError::Unavailable(
                    "package Git query timed out".into(),
                ));
            }
            self.control.check()?;
            value
        })
    }
}
