use super::*;
use crate::session::registry::PreparedRuntime;
pub(super) struct ClientFactory {
    pub inner: StoredRuntimeFactory,
    pub services: Arc<Services>,
}
impl RuntimeFactory for ClientFactory {
    fn prepare<'a>(&'a self, target: &'a Target, epoch: u64) -> PortFuture<'a, PreparedRuntime> {
        Box::pin(async move {
            let prepared = self.inner.prepare(target, epoch).await?;
            let template = self
                .services
                .templates
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .remove(target.session.as_str())
                .ok_or(DomainError::Conflict)?;
            let mut runner = template.as_ref().clone();
            if let Some(project) = &runner.adapters.project
                && let crate::projects::ProjectPort::Shared(project) =
                    &mut *project.lock().map_err(|_| DomainError::Conflict)?
            {
                *project.caller.lock().map_err(|_| DomainError::Conflict)? =
                    Arc::downgrade(prepared.runtime());
            }

            runner.adapters.delegation = Some(Arc::new(
                crate::session::subagent_tool::DelegationService::new(
                    prepared.runtime(),
                    template,
                    prepared.provider().clone(),
                    self.services.config.models.clone(),
                    tokio::runtime::Handle::current(),
                )?,
            ));
            Ok(PreparedRuntime::from_shared(
                prepared.runtime().clone(),
                prepared.provider().clone(),
                Arc::new(runner),
            ))
        })
    }
}
