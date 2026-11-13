use super::*;
use crate::github::local::{LocalGit, LocalPlan};
pub(super) struct Pending {
    service: Arc<LocalGit>,
    plan: LocalPlan,
    scope: String,
    expires: std::time::Instant,
}
impl Host {
    pub fn github_scope(&self) -> Result<String> {
        let s = self.commands.snapshot()?.scope;
        Ok(json!([
            s.workspace,
            s.session,
            self.registry
                .selected()?
                .services
                .runtime()
                .context_epoch()?
        ])
        .to_string())
    }
    pub(super) fn github_upload_preview(&self, args: &Value) -> Result<Value> {
        let request = crate::github::tool::request(args)?;
        let folders = self.folder_selection()?;
        let root = folders.resolve(
            request
                .parameters
                .get("folder")
                .and_then(Value::as_str)
                .unwrap_or("."),
            false,
        )?;
        let selected = self.registry.selected()?;
        let (grant, revision) = selected
            .services
            .tools()
            .folder_access()
            .ok_or(DomainError::NotFound)?;
        let service = self.services.github.as_ref().ok_or(DomainError::NotFound)?;
        let control = crate::resource_fs::ScanControl {
            cancel: &self.closing,
            deadline: std::time::Instant::now() + Duration::from_secs(60),
        };
        service
            .preview_upload(
                request,
                root,
                grant,
                revision,
                &self.github_scope()?,
                &control,
            )
            .map_err(crate::github::tool::domain)
    }
    pub(super) fn local_git_preview(&self, args: &Value) -> Result<Value> {
        let action = args["action"]
            .as_str()
            .ok_or(DomainError::PermissionDenied)?;
        let folders = self.folder_selection()?;
        let root = folders.resolve(
            args["folder"].as_str().unwrap_or("."),
            !["status", "diff", "branches", "recovery_status"].contains(&action),
        )?;
        let selected = self.registry.selected()?;
        let (authority, revision) = selected
            .services
            .tools()
            .folder_access()
            .ok_or(DomainError::NotFound)?;
        let t =
            self.config.tooling.as_ref().ok_or_else(|| {
                DomainError::Unavailable("Configure the native Git helper".into())
            })?;
        let writable = folders
            .resolve(
                root.root().to_str().ok_or(DomainError::PermissionDenied)?,
                true,
            )
            .is_ok();
        let mut service = LocalGit::new(
            root,
            &t.process_host,
            &t.git,
            &t.bubblewrap,
            &self.state.join("local-git"),
        )?
        .with_grant(authority, revision, writable);
        if let Some(github) = &self.services.github {
            service = service.with_github(github.clone());
        }
        let service = Arc::new(service);
        let scope = self.github_scope()?;
        let plan = service.prepare(
            action,
            args,
            &scope,
            &crate::resource_fs::ScanControl {
                cancel: &self.closing,
                deadline: std::time::Instant::now() + Duration::from_secs(30),
            },
        )?;
        let ticket = nonce()?;
        let preview = plan.preview();
        if preview.to_string().len() > 40000 {
            return Err(DomainError::LimitExceeded(
                "Local Git preview; select fewer paths".into(),
            ));
        }
        let mut plans = self
            .local_git_plans
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        plans.retain(|_, p| p.expires > std::time::Instant::now() && p.scope == scope);
        if plans.len() >= 8 {
            return Err(DomainError::LimitExceeded(
                "Pending local Git confirmations".into(),
            ));
        }
        plans.insert(
            ticket.clone(),
            Pending {
                service,
                plan,
                scope,
                expires: std::time::Instant::now() + Duration::from_secs(300),
            },
        );
        Ok(json!({"ticket":ticket,"preview":preview}))
    }
    pub(super) fn local_git_dismiss(&self, args: &Value) -> Result<Value> {
        let ticket = args["ticket"].as_str().ok_or(DomainError::NotFound)?;
        self.local_git_plans
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .remove(ticket);
        Ok(json!({"dismissed":true}))
    }
    pub(super) async fn ensure_local_git_idle(&self, args: &Value) -> Result<()> {
        let root = {
            let plans = self
                .local_git_plans
                .lock()
                .map_err(|_| DomainError::Conflict)?;
            let Some(pending) = plans.get(args["ticket"].as_str().ok_or(DomainError::NotFound)?)
            else {
                return Ok(());
            };
            pending.service.root().to_path_buf()
        };
        self.ensure_folder_idle(&root).await
    }
    pub(super) fn local_git_confirm(&self, args: &Value) -> Result<Value> {
        let pending = self
            .local_git_plans
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .remove(args["ticket"].as_str().ok_or(DomainError::NotFound)?)
            .ok_or(DomainError::NotFound)?;
        let scope = self.github_scope()?;
        if pending.scope != scope || pending.expires <= std::time::Instant::now() {
            return Err(DomainError::Conflict);
        }
        self.check_authority()?;
        self.undo_activity_check()?;
        let control = crate::resource_fs::ScanControl {
            cancel: &self.closing,
            deadline: std::time::Instant::now() + Duration::from_secs(30),
        };
        let recovery_path = pending.plan.preview["recovery_path"].clone();
        match pending.service.execute(pending.plan, &scope, &control) {
            Ok(mut result) => {
                for key in ["status", "diff", "staged", "branches"] {
                    if let Some(text) = result[key].as_str()
                        && text.len() > 12000
                    {
                        let mut end = 12000;
                        while !text.is_char_boundary(end) {
                            end -= 1;
                        }
                        result[key] = json!(format!(
                            "{}\n[Clipped; open workspace Changes for full paged review]",
                            &text[..end]
                        ));
                    }
                }
                Ok(json!({"ok":true,"result":result}))
            }
            Err(error) => Ok(
                json!({"ok":false,"error":format!("{error:?}"),"outcome":if error==DomainError::DurabilityUnknown{"unknown"}else{"not_applied"},"recovery_path":recovery_path,"retry_automatically":false}),
            ),
        }
    }
}
