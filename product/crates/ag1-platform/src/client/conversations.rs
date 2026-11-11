//! Conversation drivers share workspace/runtime leases, not selected sessions or approvals.
use super::*;
impl Host {
    pub fn owns_workspace_resources(&self) -> bool {
        self.owns_resources
    }
    pub fn conversation_status(&self) -> Result<Value> {
        let snapshot = self.commands.snapshot()?;
        let pending = self.commands.pending_approvals()?.len();
        let error = self
            .failure
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clone();
        let status = if error.is_some() || self.commands.recovery_required()? {
            "failed"
        } else if pending > 0 {
            "awaiting_input"
        } else if !snapshot.queued.is_empty() {
            if self
                .registry
                .selected()?
                .services
                .runtime()
                .waiting_for_inference()
            {
                "queued"
            } else {
                "running"
            }
        } else {
            match snapshot.last_terminal.map(|t| t.status) {
                Some(w::OperationStatus::Completed) => "completed",
                Some(w::OperationStatus::Failed) => "failed",
                _ => "idle",
            }
        };
        Ok(
            json!({"session":snapshot.scope.session,"workspace":self.workspace.root(),"status":status,"pending":pending,"error":error}),
        )
    }

    /// The desktop pool must serialize creation and retain one live owner per session.
    pub async fn open_conversation(self: &Arc<Self>, session: Option<&str>) -> Result<Arc<Self>> {
        let _workspace_guard = self.workspace_controls.lock().await;
        self.check_authority()?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(DomainError::Unavailable("Workspace is closing".into()));
        }
        let mut owners = self.conversation_owners.lock().await;
        self.check_authority()?;
        if let Some(id) = session {
            if let Some(host) = owners.get(id).and_then(std::sync::Weak::upgrade) {
                if !host.stopping.load(Ordering::Acquire)
                    && host.commands.snapshot()?.scope.session.as_str() == id
                {
                    return Ok(host);
                }
            }
        }
        let session = match session {
            Some(id) => SessionId::new(id)?,
            None => {
                let id = SessionId::new(format!("chat-{}", nonce()?))?;
                let key = nonce()?;
                self.store.bootstrap_session(SessionBootstrap {
                    command: &format!("bootstrap-{}", id.as_str()), session: id.as_str(), origin: id.as_str(), cursor_key: key.as_bytes(),
                    seed: SessionSeed::Fresh { header: &serde_json::to_vec(&json!({"type":"session","version":3,"id":id.as_str(),"cwd":self.workspace.root(),"timestamp":timestamp()})).map_err(json_error)?, records: &[] },
                })?;
                id
            }
        };
        // Validate existence before creating settings; never silently create a requested chat.
        use ag1_core::storage::StoragePort;
        self.store.journal_snapshot(&session)?;
        let previous = self
            .store
            .conversation_selection(self.workspace.id().as_str(), session.as_str())?;
        let epoch = previous
            .as_ref()
            .map_or(Some(1), |p| p.selection.epoch.checked_add(1))
            .ok_or(DomainError::Conflict)?;
        let preference_state = self
            .state
            .join("conversation-preferences")
            .join(ag1_storage::digest(session.as_str().as_bytes()));
        fs::create_dir_all(&preference_state).map_err(crate::io_error)?;
        let approval_policy =
            approval_policy::ApprovalPolicy::open(&preference_state, "interactive")?;
        let ui = Arc::new(extensions::Ui::default());
        let source = &self.services;
        let services = Arc::new(Services {
            mcp: self.services.mcp.clone(),
            github: source.github.clone(),
            working_projects: Mutex::new(Default::default()),
            folder_access: Mutex::new(Default::default()),
            approval_policy: approval_policy.clone(),
            ui: ui.clone(),
            templates: Mutex::new(Default::default()),
            imported_settings: source.imported_settings.clone(),
            workspace: self.workspace.clone(),
            store: self.store.clone(),
            config: self.config.clone(),
            provider: source.provider.clone(),
            clock: source.clock.clone(),
            state: self.state.clone(),
            project: self.project.clone(),
            scheduler: source.scheduler.clone(),
            http: source.http.clone(),
        });
        let factory = Arc::new(factory::ClientFactory {
            inner: StoredRuntimeFactory {
                store: self.store.clone(),
                pool: self.pool.clone(),
                scheduler: services.scheduler.clone(),
                http: services.http.clone(),
                services: services.clone(),
            },
            services: services.clone(),
        });
        let prepared = factory
            .prepare(
                &Target {
                    workspace: self.workspace.id().as_str().into(),
                    session: session.clone(),
                },
                epoch,
            )
            .await?;
        let registry = Arc::new(RuntimeRegistry::durable_conversation(
            prepared,
            previous,
            format!("conversation-open-{}", nonce()?),
        )?);
        registry.reconcile_selection().await?;
        let settings = &source.imported_settings;
        let commands = RuntimeCommands::new(
            registry.clone(),
            factory,
            nonce()?.try_into().map_err(DomainError::InvalidInput)?,
            source.clock.clone(),
            RunSettings {
                max_generations: self.config.max_generations,
                models: self.config.models.clone(),
                compaction: settings.compaction.clone(),
                retry: settings.retry.clone(),
                steering_mode: settings.steering.clone(),
                follow_up_mode: settings.follow_up.clone(),
                ..Default::default()
            },
            tokio::runtime::Handle::current(),
        )?;
        commands.initialize().await?;
        let host = Arc::new(Self {
            live_changed: tokio::sync::Notify::new(),
            live_revision: std::sync::atomic::AtomicU64::new(0),
            approval_policy,
            local_runtime: self.local_runtime.clone(),
            authority: self.authority.clone(),
            owns_resources: false,
            owns_runtime: false,
            closing: AtomicBool::new(false),
            workspace_closing: self.workspace_closing.clone(),
            runtime_closing: self.runtime_closing.clone(),
            shared_hosts: self.shared_hosts.clone(),
            services,
            pool: self.pool.clone(),
            conversation_owners: self.conversation_owners.clone(),
            endpoint_leases: Mutex::new(Vec::new()),
            commands,
            registry,
            store: self.store.clone(),
            workspace: self.workspace.clone(),
            config: self.config.clone(),
            state: self.state.clone(),
            ui,
            observations: Mutex::new(Default::default()),
            terminal: Mutex::new(None),
            progress: Mutex::new(String::new()),
            failure: Mutex::new(None),
            uploads: Mutex::new(Default::default()),
            diff_review: Mutex::new(None),
            local_git_plans:Mutex::new(Default::default()),
            project: self.project.clone(),
            controls: tokio::sync::Mutex::new(()),
            workspace_controls: self.workspace_controls.clone(),
            last_view: Mutex::new(None),
            wake: tokio::sync::Notify::new(),
            driver: tokio::sync::Mutex::new(None),
            stopping: AtomicBool::new(false),
        });
        let driver = host.clone();
        host.start_folder_project_poll();
        *host.driver.lock().await = Some(tokio::spawn(async move {
            driver.drive().await;
        }));
        let weak = Arc::downgrade(&host);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let Some(owner) = weak.upgrade() else { break };
                if owner.stopping.load(Ordering::Acquire) {
                    break;
                }
                let _ = owner.poll_terminal();
            }
        });
        owners.insert(session.as_str().into(), Arc::downgrade(&host));
        host.shared_hosts.lock().await.push(Arc::downgrade(&host));
        Ok(host)
    }
}
