//! Native startup binds to durable workspace authority. The P12 environment
//! configuration still explicitly selects isolated native clients.
#[cfg(feature="visual-test")]
#[path="mcp_directory_fixture.rs"]
mod mcp_directory_fixture;
use ag1_platform::client::{Config, Host};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
pub struct NativeClient {
    #[cfg(target_os = "linux")]
    mcp_oauth_entry:std::sync::Mutex<Option<Arc<dyn ag1_platform::mcp::manager::OAuthEntry>>>,
    mcp_input_entry:std::sync::Mutex<Option<Arc<dyn ag1_platform::mcp::input::Entry>>>,
    mcp_private_entry:std::sync::Mutex<Option<Arc<dyn ag1_platform::mcp::manager::PrivateEntry>>>,
    active: std::sync::atomic::AtomicBool,
    preview: std::sync::Mutex<Option<crate::project_preview::ProjectPreview>>,
    config: Option<Config>,
    legacy_default: bool,
    owner: tokio::sync::Mutex<Option<Arc<Host>>>,
    retained: tokio::sync::Mutex<Vec<Arc<Host>>>,
}
impl NativeClient {
    pub fn from_environment() -> Result<Self, String> {
        Self::from_config_path(
            std::env::var_os("AG1_NATIVE_CONFIG").map(PathBuf::from),
            true,
        )
    }
    pub fn from_desktop(state: &Path, legacy_default: bool) -> Result<Self, String> {
        let path = std::env::var_os("AG1_NATIVE_CONFIG")
            .map(PathBuf::from)
            .or_else(|| {
                (!legacy_default)
                    .then(|| state.join("native-host.json"))
                    .filter(|path| path.exists())
            });
        Self::from_config_path(path, legacy_default)
    }
    fn from_config_path(path: Option<PathBuf>, legacy_default: bool) -> Result<Self, String> {
        let config = path
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
            #[cfg(target_os = "linux")]
            mcp_oauth_entry:std::sync::Mutex::new(None),
            mcp_input_entry:std::sync::Mutex::new(None),
            mcp_private_entry:std::sync::Mutex::new(None),
            active: std::sync::atomic::AtomicBool::new(selected),
            config,
            legacy_default,
            preview: std::sync::Mutex::new(None),
            owner: tokio::sync::Mutex::new(None),
            retained: tokio::sync::Mutex::new(Vec::new()),
        })
    }
    /// Public model metadata only; selecting a model never loads its weights.
    pub fn preferences(&self) -> Value {
        serde_json::json!({"approval_mode":self.config.as_ref().map(|c|c.policy_mode.as_str()).unwrap_or("interactive")})
    }
    pub fn models(&self) -> Value {
        serde_json::json!(self.config.as_ref().map(|c| c.models.iter().map(|m| serde_json::json!({"id":m["id"],"provider":m["provider"],"contextWindow":m["contextWindow"],"input":m["input"],"reasoning":m["reasoning"],"reasoningControls":ag1_platform::provider::reasoning_controls::declared(m),"conflict":m["runtimeConflict"]})).collect::<Vec<_>>()).unwrap_or_default())
    }
    pub async fn reasoning_controls(&self, provider: &str, id: &str) -> Result<Value, String> {
        let model = self
            .config
            .as_ref()
            .and_then(|c| {
                c.models
                    .iter()
                    .find(|m| m["provider"] == provider && m["id"] == id)
            })
            .ok_or("Model is unavailable")?;
        Ok(ag1_platform::provider::reasoning_controls::discover(model).await)
    }
    pub async fn general(&self, state: &Path) -> Result<Value, String> {
        let path = state.join("general-chat");
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        match std::fs::symlink_metadata(&path) {
            Ok(meta)
                if meta.is_dir()
                    && !meta.is_symlink()
                    && meta.uid() == unsafe { libc::geteuid() }
                    && meta.mode() & 0o077 == 0 =>
            {
                ()
            }
            Ok(_) => return Err("General chat directory must be private and owned by you".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&path)
                    .map_err(|e| e.to_string())?;
            }
            Err(e) => return Err(e.to_string()),
        }
        self.open(&path, state).await
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
        Ok(!self.legacy_default
            || self.config.is_some()
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
        }
        let mut retained = self.retained.lock().await;
        if let Some(host) = retained.iter().find(|h| h.workspace.root() == canonical) {
            let value =
                serde_json::to_value(host.settled_view().await.map_err(|e| format!("{e:?}"))?)
                    .map_err(|e| e.to_string())?;
            self.close_preview()?;
            *slot = Some(host.clone());
            return Ok(value);
        }
        // Keep admission bounded without discarding any active conversation.
        Self::make_room(&mut retained, slot.as_ref()).await?;
        let parent = retained.first().cloned();
        drop(retained);
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
        let mut config: Config = match self.config.clone() {
            _ if record.as_ref().is_some_and(|r| r.transfer.is_some()) => {
                let bytes = std::fs::read(private.join("host.json")).map_err(|e| e.to_string())?;
                if bytes.len() > 1024 * 1024 {
                    return Err("Native config too large".into());
                }
                serde_json::from_slice(&bytes).map_err(|_| "Invalid migrated host configuration")?
            }
            Some(config) => config,
            None => {
                return Err(format!(
                    "Native model configuration is missing: {}",
                    state.join("native-host.json").display()
                ));
            }
        };
        refresh_default_prompt(&mut config);
        refresh_packaged_tool_helper(&mut config, &std::env::current_exe().map_err(|e| e.to_string())?)?;
        if canonical == state.join("general-chat") {
            let note = "This conversation began as a general chat. Until the user chooses a working folder in Chat folders, the default general-chat directory is private chat storage, not a codebase to inspect. The current Chat folder access section is authoritative after the user adds or changes folders. Answer directly when possible; ask for folder access only when files are needed.";
            config.prompt_append = Some(match config.prompt_append.take() {
                Some(existing) if !existing.is_empty() => format!("{existing}\n\n{note}"),
                _ => note.into(),
            });
        }
        if record.is_none() {
            ag1_platform::cutover::Registry::for_user()
                .and_then(|registry| registry.initialize_native(&canonical))
                .map_err(|e| format!("{e:?}"))?;
        }
        let github=ag1_platform::github::service::Service::shared(state).map_err(|e|e.message.to_string())?;
        #[cfg(feature = "visual-test")]
        let injected=crate::cancellation_fixture::from_environment(&config)?;
        let host = {
            #[cfg(feature = "visual-test")]
            if let Some(runtime)=injected {
                let registry=ag1_platform::cutover::Registry::for_user().map_err(|e|format!("{e:?}"))?;
                Host::open_rehearsal_with_runtime(&canonical,&private,config,&registry,runtime).await
            } else { let mcp=self.mcp_context(&config,&canonical,state)?; Host::open_with_mcp(&canonical,&private,config,github,mcp,parent.as_deref()).await }
            #[cfg(not(feature = "visual-test"))]
            { let mcp=self.mcp_context(&config,&canonical,state)?; Host::open_with_mcp(&canonical,&private,config,github,mcp,parent.as_deref()).await }
        }.map_err(|e|format!("{e:?}"))?;
        let value = serde_json::to_value(host.view().map_err(|e| format!("{e:?}"))?)
            .map_err(|e| e.to_string())?;
        if let Err(error) = self.close_preview() {
            let _ = host.shutdown().await;
            return Err(error);
        }
        self.retained.lock().await.push(host.clone());
        *slot = Some(host);
        self.active
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(value)
    }
    #[cfg(target_os = "linux")]
    pub fn set_mcp_oauth_entry(&self,entry:Arc<dyn ag1_platform::mcp::manager::OAuthEntry>)->Result<(),String> {
        *self.mcp_oauth_entry.lock().map_err(|_|"MCP OAuth entry lock")?=Some(entry);Ok(())
    }
    pub fn set_mcp_input_entry(&self,entry:Arc<dyn ag1_platform::mcp::input::Entry>)->Result<(),String> {
        *self.mcp_input_entry.lock().map_err(|_|"MCP input entry lock")?=Some(entry);Ok(())
    }
    pub fn set_mcp_private_entry(&self,entry:Arc<dyn ag1_platform::mcp::manager::PrivateEntry>)->Result<(),String> {
        *self.mcp_private_entry.lock().map_err(|_|"MCP private entry lock")?=Some(entry);Ok(())
    }
    fn mcp_context(&self,config:&Config,canonical:&Path,state:&Path)->Result<ag1_platform::client::McpServices,String> {
        let helper=config.tooling.as_ref().map(|t|t.process_host.clone()).unwrap_or(std::env::current_exe().map_err(|e|e.to_string())?.with_file_name("ag1-process-host"));
        let service=ag1_platform::mcp::service::Service::shared(state,&helper).map_err(|e|format!("{e:?}"))?;
        #[cfg(feature="visual-test")]
        let service=mcp_directory_fixture::mcp_directory(service)?;
        #[cfg(target_os = "linux")]
        if let Some(entry)=self.mcp_oauth_entry.lock().map_err(|_|"MCP OAuth entry lock")?.clone(){service.set_oauth_entry(entry).map_err(|e|format!("{e:?}"))?;}
        if let Some(entry)=self.mcp_input_entry.lock().map_err(|_|"MCP input entry lock")?.clone(){service.set_input_entry(entry).map_err(|e|format!("{e:?}"))?;}
        if let Some(entry)=self.mcp_private_entry.lock().map_err(|_|"MCP private entry lock")?.clone(){service.set_private_entry(entry).map_err(|e|format!("{e:?}"))?;}
        Ok(ag1_platform::mcp::service::Context {service,general_chat:canonical==state.join("general-chat")})
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
        let data = host
            .with_project_control(&envelope.scope, |runtime| {
                let mut preview = self
                    .preview
                    .lock()
                    .map_err(|_| ag1_core::DomainError::Conflict)?;
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
            .await
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
    async fn make_room(
        retained: &mut Vec<Arc<Host>>,
        selected: Option<&Arc<Host>>,
    ) -> Result<(), String> {
        if retained.len() < 64 {
            return Ok(());
        }
        if let Some(index) = retained.iter().position(|h| {
            !h.owns_workspace_resources()
                && selected.is_none_or(|s| !Arc::ptr_eq(s, h))
                && h.commands.snapshot().is_ok_and(|s| s.queued.is_empty())
        }) {
            retained[index]
                .shutdown()
                .await
                .map_err(|e| format!("{e:?}"))?;
            retained.remove(index);
            return Ok(());
        }
        Err("All conversation slots are busy. Wait for a response to finish before opening another chat.".into())
    }
    pub async fn select_conversation(&self, session: Option<&str>) -> Result<Value, String> {
        let mut owner = self.owner.lock().await;
        let current = owner.as_ref().ok_or("Open a project first")?.clone();
        let mut retained = self.retained.lock().await;
        let existing = if let Some(id) = session {
            retained
                .iter()
                .find(|h| {
                    h.workspace.root() == current.workspace.root()
                        && h.commands
                            .snapshot()
                            .is_ok_and(|s| s.scope.session.as_str() == id)
                })
                .cloned()
        } else {
            None
        };
        let host = if let Some(host) = existing {
            host
        } else {
            Self::make_room(&mut retained, Some(&current)).await?;
            let host = current
                .open_conversation(session)
                .await
                .map_err(|e| format!("{e:?}"))?;
            retained.push(host.clone());
            host
        };
        let value = serde_json::to_value(host.settled_view().await.map_err(|e| format!("{e:?}"))?)
            .map_err(|e| e.to_string())?;
        self.close_preview()?;
        *owner = Some(host);
        Ok(value)
    }
    pub async fn host_for(&self, scope: &ag1_protocol::Scope) -> Result<Arc<Host>, String> {
        self.retained
            .lock()
            .await
            .iter()
            .find(|h| {
                h.commands.snapshot().is_ok_and(|s| {
                    s.scope.workspace == scope.workspace && s.scope.instance == scope.instance
                })
            })
            .cloned()
            .ok_or("The requested conversation is no longer open".into())
    }
    pub async fn conversation_statuses(&self) -> Result<Value, String> {
        let hosts = self.retained.lock().await.clone();
        let mut rows = Vec::new();
        for host in hosts {
            rows.push(host.conversation_status().map_err(|e| format!("{e:?}"))?);
        }
        Ok(serde_json::json!(rows))
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
        // Detach hosts before awaiting them. A failed provider can leave a
        // host's shutdown future waiting on the driver; holding the client
        // locks across that await would also freeze view/quit commands.
        let owner = self.owner.lock().await.take();
        let mut hosts = {
            let mut retained = self.retained.lock().await;
            std::mem::take(&mut *retained)
        };
        if let Some(owner) = owner
            && !hosts.iter().any(|host| Arc::ptr_eq(host, &owner))
        {
            hosts.push(owner);
        }
        for host in hosts.into_iter().rev() {
            // Shutdown is best effort during application exit. The process
            // must still be closable when a backend has stopped responding.
            let _ = tokio::time::timeout(Duration::from_secs(8), host.shutdown()).await;
        }
        self.close_preview()?;
        self.active
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
}

/// Installed helper paths belong to the currently running release. Saved
/// settings can outlive a removed release; only rebase our own package layout.
/// The separately captured model/runtime supervisor is deliberately untouched.
fn refresh_packaged_tool_helper(config: &mut Config, executable: &Path) -> Result<(), String> {
    fn installation(path: &Path, name: &str) -> Option<PathBuf> {
        if path.file_name()? != name || path.parent()?.file_name()? != "bin" {
            return None;
        }
        let releases = path.parent()?.parent()?.parent()?;
        (releases.file_name()? == "releases").then(|| releases.parent().map(Path::to_path_buf)).flatten()
    }
    let Some(tooling) = &mut config.tooling else { return Ok(()); };
    let Some(root) = installation(executable, "ag1-desktop") else { return Ok(()); };
    if installation(&tooling.process_host, "ag1-process-host").as_ref() != Some(&root) {
        return Ok(());
    }
    let helper = executable.with_file_name("ag1-process-host");
    if !helper.is_file() {
        return Err("The installed AG1 process helper is missing. Reinstall the complete AG1 package.".into());
    }
    tooling.process_host = helper;
    Ok(())
}

/// Retire only the exact default captured for the original desktop migration.
/// Never detect user instructions by wording or strip arbitrary custom prompts.
fn refresh_default_prompt(config: &mut Config) {
    use sha2::{Digest, Sha256};
    if format!("{:x}", Sha256::digest(config.system_prompt.as_bytes()))
        == "e48c75aaae786f4f3c2c8baa0964967063ccb3b3c3aa72affed44ace5bba9d46"
    {
        config.system_prompt.clear();
    }
}

#[cfg(test)]
mod prompt_migration_tests {
    use super::*;
    #[test]
    fn only_exact_captured_default_is_retired_and_custom_instructions_survive() {
        let captured = include_str!("../../../../../fixtures/context/desktop-legacy-prompt.txt");
        let mut config: Config = serde_json::from_value(serde_json::json!({"models":[],"selected":"fixture","credential_env":null,"system_prompt":captured,"fixture":true})).unwrap();
        refresh_default_prompt(&mut config);
        assert!(config.system_prompt.is_empty());
        for custom in [
            "You are a custom assistant.",
            &format!("{captured}\nKeep my custom instruction."),
        ] {
            config.system_prompt = custom.into();
            refresh_default_prompt(&mut config);
            assert_eq!(config.system_prompt, custom);
        }
    }
}

#[cfg(test)]
mod packaged_helper_tests {
    use super::*;
    fn config(helper: &Path) -> Config {
        serde_json::from_value(serde_json::json!({"models":[],"selected":"fixture","system_prompt":"custom","fixture":true,
            "tooling":{"process_host":helper,"git":"/usr/bin/git","rg":"/usr/bin/rg","fd":"/usr/bin/fd","bubblewrap":"/usr/bin/bwrap","targets":{"mounts":[]}}})).unwrap()
    }
    #[test]
    fn removed_release_helper_rebases_and_opens_the_real_mcp_manager() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("app/releases/current/bin/ag1-desktop");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        let helper = executable.with_file_name("ag1-process-host");
        std::fs::copy("/usr/bin/true", &helper).unwrap();
        let old = dir.path().join("app/releases/removed/bin/ag1-process-host");
        let mut config = config(&old);
        let state = dir.path().join("state");std::fs::create_dir(&state).unwrap();
        assert!(matches!(ag1_platform::mcp::service::Service::shared(&state,&old),Err(ag1_core::DomainError::NotFound)));
        refresh_packaged_tool_helper(&mut config,&executable).unwrap();
        assert_eq!(config.tooling.as_ref().unwrap().process_host,helper);
        assert_eq!(config.system_prompt,"custom");
        let service = ag1_platform::mcp::service::Service::shared(&state,&config.tooling.unwrap().process_host).unwrap();
        let scope = ag1_core::tools::registry::Scope {workspace:"fixture".into(),session:"session".into(),epoch:1,revision:0};
        let session = service.session(ag1_platform::mcp::catalog::CatalogScope::GeneralChat,scope.clone(),None).unwrap();
        let list = session.dispatch(&scope,&serde_json::json!({"action":"list"})).unwrap();
        assert_eq!(list["servers"],serde_json::json!([]));
        assert_eq!(list["scope"]["kind"],"generalChat");
    }
    #[test]
    fn explicit_helpers_other_installations_and_missing_current_payload_are_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("app/releases/current/bin/ag1-desktop");
        for custom in [dir.path().join("custom/ag1-process-host"),dir.path().join("other/releases/old/bin/ag1-process-host")] {
            let mut config=config(&custom);refresh_packaged_tool_helper(&mut config,&executable).unwrap();
            assert_eq!(config.tooling.unwrap().process_host,custom);
        }
        let mut config=config(&dir.path().join("app/releases/old/bin/ag1-process-host"));
        assert!(refresh_packaged_tool_helper(&mut config,&executable).unwrap_err().contains("Reinstall"));
    }
}
