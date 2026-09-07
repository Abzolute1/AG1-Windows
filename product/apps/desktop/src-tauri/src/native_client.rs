//! Native startup binds to durable workspace authority. The P12 environment
//! configuration still explicitly selects isolated native clients.
use ag1_platform::client::{Config, Host};
use serde_json::Value;
use std::{path::Path, sync::Arc};
pub struct NativeClient {
    active: std::sync::atomic::AtomicBool,
    preview: std::sync::Mutex<Option<crate::project_preview::ProjectPreview>>,
    config: Option<Config>,
    owner: tokio::sync::Mutex<Option<Arc<Host>>>,
}
impl NativeClient {
    pub fn from_environment() -> Result<Self, String> {
        let config = std::env::var_os("AG1_NATIVE_CONFIG")
            .map(|path| -> Result<Config, String> {
                let bytes =
                    std::fs::read(path).map_err(|_| "Cannot read private native host config")?;
                if bytes.len() > 1024 * 1024 {
                    return Err("Native config too large".into());
                }
                serde_json::from_slice(&bytes).map_err(|_| "Invalid native host config".into())
            })
            .transpose()?;
        let selected = config.is_some();
        Ok(Self {
            active: std::sync::atomic::AtomicBool::new(selected),
            config,
            preview: std::sync::Mutex::new(None),
            owner: tokio::sync::Mutex::new(None),
        })
    }
    pub fn enabled(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn selects(&self, path: &Path) -> Result<bool, String> {
        let registry = ag1_platform::cutover::Registry::for_user().map_err(|e| format!("{e:?}"))?;
        let recorded = registry.inspect(path).map_err(|e| format!("{e:?}"))?;
        if recorded.as_ref().is_some_and(|r| {
            r.owner == ag1_platform::cutover::Owner::TypeScript && r.rollback.is_some()
        }) {
            return Ok(false);
        }
        Ok(self.config.is_some()
            || recorded.is_some_and(|r| {
                r.owner == ag1_platform::cutover::Owner::Native && r.transfer.is_some()
            }))
    }
    pub async fn open(&self, path: &Path, state: &Path) -> Result<Value, String> {
        let mut slot = self.owner.lock().await;
        let canonical = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        if let Some(owner) = slot.as_ref() {
            if owner.workspace.root() == canonical {
                return serde_json::to_value(owner.view().map_err(|e| format!("{e:?}"))?)
                    .map_err(|e| e.to_string());
            }
            if owner.view().map_err(|e| format!("{e:?}"))?.snapshot.status != "idle" {
                return Err("Cancel and drain active work before switching projects".into());
            }
        }
        use sha2::{Digest, Sha256};
        let fallback = state.join("native").join(format!(
            "{:x}",
            Sha256::digest(canonical.as_os_str().as_encoded_bytes())
        ));
        let record = ag1_platform::cutover::Registry::for_user()
            .map_err(|e| format!("{e:?}"))?
            .inspect(&canonical)
            .map_err(|e| format!("{e:?}"))?;
        let private = record
            .as_ref()
            .and_then(|r| r.state.clone())
            .unwrap_or(fallback);
        let config = match self.config.clone() {
            Some(config) => config,
            None if record.as_ref().is_some_and(|r| r.transfer.is_some()) => {
                let bytes = std::fs::read(private.join("host.json")).map_err(|e| e.to_string())?;
                if bytes.len() > 1024 * 1024 {
                    return Err("Native config too large".into());
                }
                serde_json::from_slice(&bytes).map_err(|_| "Invalid migrated host configuration")?
            }
            None => return Err("Native mode needs explicit configuration".into()),
        };
        if let Some(owner) = slot.as_ref() {
            owner.shutdown().await.map_err(|e| format!("{e:?}"))?;
        }
        self.close_preview()?;
        *slot = None;
        self.active
            .store(false, std::sync::atomic::Ordering::Release);
        let host = Host::open(&canonical, &private, config)
            .await
            .map_err(|e| format!("{e:?}"))?;
        let value = serde_json::to_value(host.view().map_err(|e| format!("{e:?}"))?)
            .map_err(|e| e.to_string())?;
        if let Err(error) = self.close_preview() {
            let _ = host.shutdown().await;
            return Err(error);
        }
        *slot = Some(host);
        self.active
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(value)
    }
    pub fn close_preview(&self) -> Result<(), String> {
        let mut preview = self.preview.lock().map_err(|_| "Preview lock")?;
        if let Some(window) = preview.as_mut() {
            window.close()?;
        }
        *preview = None;
        Ok(())
    }
    pub async fn preview(
        &self,
        app: &tauri::AppHandle,
        envelope: ag1_protocol::CommandEnvelope,
    ) -> Result<Value, String> {
        use ag1_protocol as w;
        w::encode(&w::WireMessage::Command(envelope.clone())).map_err(|e| format!("{e:?}"))?;
        let owner = self.owner.lock().await;
        let host = owner.as_ref().ok_or("Open a project first")?;
        let w::Command::Project { action, .. } = &envelope.command else {
            return Err("Invalid preview action".into());
        };
        let mut preview = self.preview.lock().map_err(|_| "Preview lock")?;
        let data = host
            .with_project(&envelope.scope, |runtime| {
                let error = |e: String| ag1_core::DomainError::Unavailable(e);
                match action.as_str() {
                    "preview_open" => {
                        if let Some(window) = preview.as_mut() {
                            window.close().map_err(error)?;
                        }
                        *preview = Some(
                            crate::project_preview::ProjectPreview::open(app, runtime)
                                .map_err(error)?,
                        );
                    }
                    "preview_focus" => preview
                        .as_ref()
                        .ok_or(ag1_core::DomainError::NotFound)?
                        .focus(runtime)
                        .map_err(error)?,
                    "preview_close" => {
                        if let Some(window) = preview.as_mut() {
                            window.close().map_err(error)?;
                        }
                        *preview = None;
                    }
                    _ => return Err(ag1_core::DomainError::NotFound),
                }
                Ok(serde_json::json!({"open":preview.is_some()}))
            })
            .map_err(|e| format!("{e:?}"))?;
        serde_json::to_value(w::BridgeFrame::Reply {
            id: envelope.id,
            scope: envelope.scope,
            revision: host
                .commands
                .snapshot()
                .map_err(|e| format!("{e:?}"))?
                .revision,
            durable: false,
            status: "query".into(),
            data,
        })
        .map_err(|e| e.to_string())
    }
    pub async fn host(&self) -> Result<Arc<Host>, String> {
        self.owner
            .lock()
            .await
            .clone()
            .ok_or("Open an isolated project first".into())
    }
    pub async fn view(&self) -> Result<Value, String> {
        match self.owner.lock().await.as_ref() {
            Some(host) => {
                let view = host.settled_view().await.map_err(|e| format!("{e:?}"))?;
                let mut preview = self.preview.lock().map_err(|_| "Preview lock")?;
                if let Some(window) = preview.as_mut() {
                    let valid = host.with_project(&view.snapshot.scope, |runtime| {
                        runtime.verify_preview(&window.grant)
                    });
                    if valid.is_err() && !matches!(valid, Err(ag1_core::DomainError::QueueFull)) {
                        window.close()?;
                        *preview = None;
                    }
                }
                serde_json::to_value(view).map_err(|e| e.to_string())
            }
            None => Ok(serde_json::json!({"snapshot":null,"error":null,"progress":""})),
        }
    }
    pub async fn shutdown(&self) -> Result<(), String> {
        let mut slot = self.owner.lock().await;
        if let Some(host) = slot.as_ref() {
            host.shutdown().await.map_err(|e| format!("{e:?}"))?;
        }
        self.close_preview()?;
        *slot = None;
        self.active
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
}
