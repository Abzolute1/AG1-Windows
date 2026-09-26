use super::*;
impl Host {
    pub(super) fn start_folder_project_poll(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let Some(owner) = weak.upgrade() else { break };
                if owner.stopping.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(_control) = owner.controls.try_lock()
                    && let Ok(project) = owner.working_project()
                    && let Ok(mut slot) = project.try_lock()
                    && let Some(runtime) = slot.as_mut()
                {
                    let _ = projects::poll(
                        runtime,
                        &crate::resource_fs::ScanControl {
                            cancel: &AtomicBool::new(false),
                            deadline: std::time::Instant::now() + Duration::from_millis(80),
                        },
                    );
                }
            }
        });
    }
    pub(super) fn folder_selection(&self) -> Result<crate::folder_access::Snapshot> {
        let selected = self.registry.selected()?;
        let (access, revision) = selected
            .services
            .tools()
            .folder_access()
            .ok_or(DomainError::NotFound)?;
        access.check(revision)?;
        access.snapshot()
    }
    pub(super) fn working_workspace(&self) -> Result<Arc<Workspace>> {
        self.folder_selection()?.resolve(".", false)
    }
    pub fn working_folder(&self, write: bool) -> Result<Arc<Workspace>> {
        self.folder_selection()?.resolve(".", write)
    }
    pub(super) fn working_project(&self) -> Result<Arc<Mutex<Option<crate::projects::Runtime>>>> {
        let workspace = self.working_workspace()?;
        if workspace.root() == self.workspace.root() {
            return Ok(self.project.clone());
        }
        let session = self.commands.snapshot()?.scope.session;
        let key = ag1_storage::digest(
            format!("{}:{}", session.as_str(), workspace.root().display()).as_bytes(),
        );
        self.services
            .working_projects
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .get(&key)
            .cloned()
            .ok_or(DomainError::Conflict)
    }
    /// Caller holds host/workspace admission locks and validates the exact UI scope.
    pub(super) async fn folder_access_command(self: &Arc<Self>, args: &Value) -> Result<Value> {
        let selected = self.registry.selected()?;
        let (access, _) = selected
            .services
            .tools()
            .folder_access()
            .ok_or(DomainError::NotFound)?;
        if args.get("action").is_some() {
            if self.terminal.lock().map_err(|_|DomainError::Conflict)?.is_some() {
                return Err(DomainError::Unavailable("Close this chat's terminal before changing folders".into()));
            }
            if !self.commands.snapshot()?.queued.is_empty()
                || !selected.services.runtime().settled()?
                || self.commands.client_transition_pending()?
            {
                return Err(DomainError::Unavailable(
                    "Finish or stop the current response before changing chat folders".into(),
                ));
            }
            let activity = selected.services.tools().client_activity()?;
            if activity["jobs"].as_array().is_some_and(|jobs| {
                jobs.iter().any(|j| {
                    !matches!(
                        j["status"].as_str(),
                        Some("completed" | "failed" | "cancelled" | "exited")
                    )
                })
            }) {
                return Err(DomainError::Unavailable(
                    "Stop background jobs before changing chat folders".into(),
                ));
            }
            // A live target is owned work, not an incidental side effect of switching cwd.
            if let Ok(project) = self.working_project() {
                if project
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .as_ref()
                    .is_some_and(|p| !p.observed_pids().is_empty())
                {
                    return Err(DomainError::Unavailable(
                        "Stop the running project target before changing chat folders".into(),
                    ));
                }
            }
            let change = serde_json::from_value(args.clone()).map_err(json_error)?;
            access.change(change)?;
            self.commands.refresh_capabilities().await.map_err(|error|DomainError::Unavailable(format!("Folder permissions were saved, but this chat could not reload: {error:?}. Reopen AG1 before continuing; old tool permissions cannot be used.")))?;
            self.release_unused_working_projects(false)?;
        }
        serde_json::to_value(access.snapshot()?.selection).map_err(json_error)
    }
    pub(super) fn release_unused_working_projects(&self, all: bool) -> Result<()> {
        let current = if all {
            None
        } else {
            self.working_project().ok()
        };
        let mut projects = self
            .services
            .working_projects
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        for project in projects.values() {
            if current
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, project))
            {
                continue;
            }
            let mut slot = project.lock().map_err(|_| DomainError::Conflict)?;
            if let Some(runtime) = slot.as_mut() {
                runtime.close_private_display()?;
            }
            slot.take();
        }
        projects.retain(|_, project| {
            current
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, project))
        });
        Ok(())
    }
}
