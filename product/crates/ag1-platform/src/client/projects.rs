use super::*;
use crate::resource_fs::ScanControl;
use std::time::Instant;
pub(super) fn load(
    workspace: &Arc<Workspace>,
    state: &Path,
    tooling: Option<&tooling::Tooling>,
) -> Result<Option<crate::projects::Runtime>> {
    let Some(tooling) = tooling else {
        return Ok(None);
    };
    let private = state.join("project");
    fs::create_dir_all(&private).map_err(crate::io_error)?;
    let mut runtime = crate::projects::Runtime::load(
        workspace.clone(),
        WorkspaceService::new()?.register(&private)?,
        &tooling.process_host,
        tooling.targets.clone(),
    )?;
    runtime.bind_git_source(&tooling.process_host, &tooling.git)?;
    Ok(Some(runtime))
}
pub(super) fn poll(
    runtime: &mut crate::projects::Runtime,
    control: &ScanControl<'_>,
) -> Result<()> {
    match runtime.poll(control) {
        Ok(record)
            if matches!(
                record.lifecycle,
                ag1_core::projects::Lifecycle::Exited | ag1_core::projects::Lifecycle::Failed
            ) =>
        {
            runtime.finish(control)?;
            Ok(())
        }
        Ok(_) | Err(DomainError::NotFound) => Ok(()),
        Err(error) => Err(error),
    }
}
impl Host {
    /// Explicit native preview actions share the command/poll exclusion gate.
    /// Waiting is asynchronous; scope and authority are checked after admission.
    /// The callback executes once, with no retry of a partially performed effect.
    pub async fn with_project_control<T>(
        &self,
        scope: &w::Scope,
        f: impl FnOnce(&crate::projects::Runtime) -> Result<T>,
    ) -> Result<T> {
        let _control = self.controls.lock().await;
        self.with_project(scope, f)
    }
    /// Host-only capability access for native preview windows. The renderer can
    /// select an action, but cannot supply a runtime, process, origin or grant.
    pub fn with_project<T>(
        &self,
        scope: &w::Scope,
        f: impl FnOnce(&crate::projects::Runtime) -> Result<T>,
    ) -> Result<T> {
        self.check_authority()?;
        if self.commands.snapshot()?.scope != *scope || self.commands.client_transition_pending()? {
            return Err(DomainError::Conflict);
        }
        let slot = self
            .project
            .try_lock()
            .map_err(|_| DomainError::QueueFull)?;
        f(slot
            .as_ref()
            .ok_or_else(|| DomainError::Unavailable("Project runtime is not configured".into()))?)
    }

    pub(super) fn project_command(&self, action: &str, args: &Value) -> Result<Value> {
        let mut slot = self
            .project
            .try_lock()
            .map_err(|_| DomainError::QueueFull)?;
        let Some(runtime) = slot.as_mut() else {
            return if action == "status" {
                Ok(
                    json!({"available":false,"reason":"Configure the native process helper and target toolchains in the host configuration","targets":[],"history":[]}),
                )
            } else {
                Err(DomainError::Unavailable(
                    "Project toolchains are not configured".into(),
                ))
            };
        };
        let cancel = AtomicBool::new(false);
        let control = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        match action {
            "status" => {
                poll(runtime, &control)?;
                Ok(
                    json!({"available":true,"targets":runtime.project.targets.iter().map(|target|{let mut value=json!(target);for action in ["install","build","run","dev","check"] {value["controls"][action]=match runtime.command(target.kind,action){Ok(_)=>json!({"available":true}),Err(error)=>json!({"available":false,"reason":format!("{error:?}")})};}value}).collect::<Vec<_>>(),"history":runtime.history()?.into_iter().rev().take(30).collect::<Vec<_>>(),"preview":match runtime.preview_grant(){Ok(grant)=>json!({"available":true,"operation":grant.operation}),Err(error)=>json!({"available":false,"reason":format!("{error:?}")})},"revision":runtime.revision()?}),
                )
            }
            "discover" => {
                runtime.discover()?;
                Ok(json!(runtime.project))
            }
            "start" => {
                let target = args["target"].as_str().ok_or(DomainError::NotFound)?;
                let action = args["operation"].as_str().ok_or(DomainError::NotFound)?;
                if args["revision"] != runtime.revision()? {
                    return Err(DomainError::Conflict);
                }
                let kind = runtime
                    .project
                    .targets
                    .iter()
                    .find(|t| t.id == target)
                    .ok_or(DomainError::NotFound)?
                    .kind;
                let mut commands = runtime.command(kind, action)?;
                let readiness = if action == "run"
                    && matches!(
                        kind,
                        ag1_core::projects::Kind::Web | ag1_core::projects::Kind::Tauri
                    ) {
                    let package: Value = serde_json::from_slice(
                        &self.workspace.snapshot("package.json", 1024 * 1024)?.bytes,
                    )
                    .map_err(json_error)?;
                    if package["scripts"]["dev"]
                        .as_str()
                        .is_some_and(|s| s.split_whitespace().next() == Some("vite"))
                    {
                        let port = if kind == ag1_core::projects::Kind::Tauri {
                            let config: Value = serde_json::from_slice(
                                &self
                                    .workspace
                                    .snapshot("src-tauri/tauri.conf.json", 1024 * 1024)?
                                    .bytes,
                            )
                            .map_err(json_error)?;
                            let url = reqwest::Url::parse(
                                config["build"]["devUrl"]
                                    .as_str()
                                    .ok_or(DomainError::NotFound)?,
                            )
                            .map_err(|_| DomainError::PermissionDenied)?;
                            if url.scheme() != "http"
                                || !matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
                            {
                                return Err(DomainError::PermissionDenied);
                            }
                            url.port_or_known_default().ok_or(DomainError::NotFound)?
                        } else {
                            let socket = std::net::TcpListener::bind("127.0.0.1:0")
                                .map_err(crate::io_error)?;
                            let port = socket.local_addr().map_err(crate::io_error)?.port();
                            commands[0].argv.extend([
                                "--".into(),
                                "--host".into(),
                                "127.0.0.1".into(),
                                "--port".into(),
                                port.to_string(),
                                "--strictPort".into(),
                            ]);
                            port
                        };
                        // Vite serves package.json without HTML transforms. Match
                        // the complete selected manifest, in addition to Runtime's
                        // owned-listener check and strict-port launch.
                        let identity = self
                            .workspace
                            .snapshot("package.json", 8192)
                            .ok()
                            .and_then(|file| String::from_utf8(file.bytes).ok());
                        identity.map(|expected_body| crate::projects::ReadyHttp {
                            address: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                            path: "/package.json".into(),
                            expected_body,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let id = runtime.start(
                    target,
                    action,
                    crate::projects::Launch {
                        commands,
                        readiness,
                        timeout_ms: 300000,
                    },
                    &control,
                )?;
                Ok(json!({"operation":id}))
            }
            "stop" => Ok(json!(runtime.stop()?)),
            "log" => {
                let operation = args["operation"].as_str().ok_or(DomainError::NotFound)?;
                let offset = args["offset"]
                    .as_str()
                    .unwrap_or("0")
                    .parse::<u64>()
                    .map_err(|_| DomainError::InvalidInput("log byte offset".into()))?;
                let bytes = runtime.log(
                    operation,
                    args["stream"].as_u64().unwrap_or(0) as usize,
                    offset,
                    32768,
                )?;
                Ok(
                    json!({"text":String::from_utf8_lossy(&bytes),"next":(offset+bytes.len() as u64).to_string()}),
                )
            }
            _ => Err(DomainError::Unavailable(format!("project.{action}"))),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
