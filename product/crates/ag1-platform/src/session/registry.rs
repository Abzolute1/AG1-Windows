//! Prepared runtime replacement for native embeddings. This owns the selected
//! session and its scoped services; P02 command admission/queues are a separate
//! layer and must not treat a returned session handle as an idempotent command.
use super::{NativeSession, ProviderResolver, ToolRunner, lifecycle::Frozen};
use ag1_core::{DomainError, PortFuture, SessionId, storage::StoragePort};
use std::sync::{Arc, Mutex};

mod durable;
mod transition;
pub use transition::{SessionAction, SessionRequest, TransitionOutcome};

/// Host capability construction receives the actual selected branch, including
/// model/tool/thinking entries written before the first user message. It must
/// not recover background orphans while a previous owner could still be live.
pub struct ServiceInput<'a> {
    pub target: &'a Target,
    pub epoch: u64,
    pub journal: &'a ag1_core::storage::StoredJournal,
}
pub struct ScopedServices {
    pub workspace: Arc<crate::workspace::Workspace>,
    pub origin: String,
    pub configuration: ag1_core::runtime::session::Configuration,
    pub provider: Arc<dyn ProviderResolver>,
    pub tools: Arc<dyn ToolRunner>,
}
pub trait ServiceFactory: Send + Sync {
    fn prepare<'a>(&'a self, input: ServiceInput<'a>) -> PortFuture<'a, ScopedServices>;
}
/// Concrete durable resume factory. Creation/forking first use the atomic P05
/// bootstrap API; this factory never creates a second scheduler or model client.
pub struct StoredRuntimeFactory {
    pub store: Arc<ag1_storage::Store>,
    pub pool: Arc<crate::executor::BlockingPool>,
    pub scheduler: ag1_core::inference::Scheduler,
    pub http: crate::provider::http::ProviderHttp,
    pub services: Arc<dyn ServiceFactory>,
}
impl RuntimeFactory for StoredRuntimeFactory {
    fn prepare<'a>(&'a self, target: &'a Target, epoch: u64) -> PortFuture<'a, PreparedRuntime> {
        Box::pin(async move {
            let session = target.session.clone();
            let store = self.store.clone();
            let journal = self
                .pool
                .submit(move || store.journal_snapshot(&session))
                .await?;
            let services = self
                .services
                .prepare(ServiceInput {
                    target,
                    epoch,
                    journal: &journal,
                })
                .await?;
            if services.workspace.id().as_str() != target.workspace {
                return Err(DomainError::PermissionDenied);
            }
            let session = target.session.clone();
            let store = self.store.clone();
            let pool = self.pool.clone();
            let scheduler = self.scheduler.clone();
            let http = self.http.clone();
            self.pool
                .submit(move || {
                    let state = ag1_core::runtime::SessionRuntime::resume(
                        session,
                        epoch,
                        &journal,
                        services.configuration,
                    )?;
                    let runtime = NativeSession::new(
                        state,
                        store,
                        services.origin,
                        services.workspace,
                        pool,
                        scheduler,
                        http,
                    )?;
                    Ok(PreparedRuntime::new(
                        runtime,
                        services.provider,
                        services.tools,
                    ))
                })
                .await
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub workspace: String,
    pub session: SessionId,
}
/// A factory must prepare fresh, unused services without dispatching inference
/// or tools. Drop must release its owned preparations, including on cancellation.
/// Model resolution uses the target journal, not the previous session's caches.
pub trait RuntimeFactory: Send + Sync {
    fn prepare<'a>(&'a self, target: &'a Target, epoch: u64) -> PortFuture<'a, PreparedRuntime>;
}
pub struct PreparedRuntime {
    runtime: Arc<NativeSession>,
    provider: Arc<dyn ProviderResolver>,
    tools: Arc<dyn ToolRunner>,
}
impl PreparedRuntime {
    pub fn new(
        runtime: NativeSession,
        provider: Arc<dyn ProviderResolver>,
        tools: Arc<dyn ToolRunner>,
    ) -> Self {
        Self::from_shared(Arc::new(runtime), provider, tools)
    }
    /// Preserve the native owner's identity when scoped adapters already hold
    /// weak references to it (for example Task). Admission is still fenced and
    /// validated by the registry and command facade.
    pub fn from_shared(
        runtime: Arc<NativeSession>,
        provider: Arc<dyn ProviderResolver>,
        tools: Arc<dyn ToolRunner>,
    ) -> Self {
        Self {
            runtime,
            provider,
            tools,
        }
    }
    pub fn runtime(&self) -> &Arc<NativeSession> {
        &self.runtime
    }
    pub fn provider(&self) -> &Arc<dyn ProviderResolver> {
        &self.provider
    }
    pub fn tools(&self) -> &Arc<dyn ToolRunner> {
        &self.tools
    }
}
#[derive(Clone)]
pub struct Selection {
    /// Registry generation, distinct from a session's compaction context epoch.
    pub epoch: u64,
    pub services: Arc<PreparedRuntime>,
}
pub struct RuntimeRegistry {
    selected: Mutex<Selection>,
    replacing: tokio::sync::Mutex<()>,
    durable: Option<durable::Durability>,
}
struct PreparedFence(Option<Frozen>);
impl PreparedFence {
    fn activate(mut self) {
        drop(self.0.take());
    }
}
impl Drop for PreparedFence {
    fn drop(&mut self) {
        if let Some(fence) = self.0.take() {
            let _ = fence.retire();
        }
    }
}
impl RuntimeRegistry {
    pub(in crate::session) fn has_durable_selection(&self) -> bool {
        self.durable.is_some()
    }
    pub fn new(initial: PreparedRuntime, epoch: u64) -> Result<Self, DomainError> {
        let frozen = initial.runtime.freeze()?;
        if !frozen.idle()?
            || !initial.runtime.settled()?
            || !initial.runtime.persistence_healthy()?
        {
            return Err(DomainError::Conflict);
        }
        drop(frozen);
        Ok(Self {
            selected: Mutex::new(Selection {
                epoch,
                services: Arc::new(initial),
            }),
            replacing: tokio::sync::Mutex::new(()),
            durable: None,
        })
    }
    pub fn selected(&self) -> Result<Selection, DomainError> {
        // Keep this lock through the selection read: pending publication and
        // selection visibility must be one admission decision.
        let durable = self.durable.as_ref().map(|d| d.lock()).transpose()?;
        if durable
            .as_ref()
            .is_some_and(|state| state.pending.is_some())
        {
            return Err(DomainError::Unavailable(
                "runtime selection requires acknowledgement recovery".into(),
            ));
        }
        self.selected
            .lock()
            .map(|s| s.clone())
            .map_err(|_| DomainError::Conflict)
    }
    /// Preparation failure leaves the old runtime untouched. Once prepared,
    /// freeze admission, cancel and drain old work, revalidate target storage,
    /// and publish exactly one new registry epoch. No process cwd is changed.
    pub async fn replace(
        &self,
        expected_epoch: u64,
        target: Target,
        factory: &dyn RuntimeFactory,
    ) -> Result<Selection, DomainError> {
        self.replace_inner(expected_epoch, target, factory, false, None)
            .await
    }
    /// Rebuild capabilities without changing the conversation or journal identity.
    pub(crate) async fn refresh(&self, factory: &dyn RuntimeFactory) -> Result<Selection, DomainError> {
        let _replacement = self.replacing.try_lock().map_err(|_|DomainError::Conflict)?;
        let previous = self.selected()?;
        let runtime = previous.services.runtime();
        if !runtime.settled()? { return Err(DomainError::Conflict); }
        let target = Target { workspace: runtime.workspace.clone(), session: runtime.state()?.stamp().session().clone() };
        self.replace_locked(previous.epoch, target, factory, false, Some(runtime.journal_revision()?)).await
    }
    /// User-directed session switching cancels unstarted prompts with durable
    /// terminal receipts. Never discard an accepted queue or replay a claim.
    pub async fn replace_cancelling_pending(
        &self,
        expected_epoch: u64,
        target: Target,
        factory: &dyn RuntimeFactory,
    ) -> Result<Selection, DomainError> {
        self.replace_inner(expected_epoch, target, factory, true, None)
            .await
    }
    async fn replace_inner(
        &self,
        expected_epoch: u64,
        target: Target,
        factory: &dyn RuntimeFactory,
        cancel_pending: bool,
        source_revision: Option<u64>,
    ) -> Result<Selection, DomainError> {
        if self.durable.as_ref().is_some_and(|d| d.conversation) {
            return Err(DomainError::Unavailable("Use the desktop chat picker to open a different conversation".into()));
        }

        let _replacement = self
            .replacing
            .try_lock()
            .map_err(|_| DomainError::Conflict)?;
        self.replace_locked(
            expected_epoch,
            target,
            factory,
            cancel_pending,
            source_revision,
        )
        .await
    }
    async fn replace_locked(
        &self,
        expected_epoch: u64,
        target: Target,
        factory: &dyn RuntimeFactory,
        cancel_pending: bool,
        source_revision: Option<u64>,
    ) -> Result<Selection, DomainError> {
        let previous = self.selected()?;
        if previous.epoch != expected_epoch {
            return Err(DomainError::Conflict);
        }
        let epoch = expected_epoch.checked_add(1).ok_or(DomainError::Conflict)?;
        let next = factory.prepare(&target, epoch).await?;
        let prepared = next.runtime.freeze()?;
        if !prepared.idle()? || !next.runtime.settled()? || !next.runtime.persistence_healthy()? {
            return Err(DomainError::Conflict);
        }
        let prepared = PreparedFence(Some(prepared));
        if self.durable.is_some()
            && (!Arc::ptr_eq(&previous.services.runtime.store, &next.runtime.store)
                || previous.services.runtime.workspace != next.runtime.workspace)
        {
            return Err(DomainError::PermissionDenied);
        }
        {
            let state = next.runtime.state()?;
            if state.stamp().session() != &target.session
                || next.runtime.workspace != target.workspace
            {
                return Err(DomainError::Conflict);
            }
        }
        let old = previous.services.runtime.freeze()?;
        // Any failure or dropped future thaws the old owner. The unused next
        // owner is inaccessible and released together with its scoped services.
        old.drain().await?;
        let previous_runtime = previous.services.runtime();
        if source_revision
            .is_some_and(|revision| previous_runtime.journal_revision().ok() != Some(revision))
        {
            return Err(DomainError::Conflict);
        }
        let commands = previous_runtime
            .command_owner
            .get()
            .and_then(std::sync::Weak::upgrade);
        if previous_runtime
            .commands_claimed
            .load(std::sync::atomic::Ordering::Acquire)
            && (commands.is_none()
                || (!cancel_pending
                    && !commands
                        .as_ref()
                        .map(|owner| owner.replacement_ready())
                        .transpose()?
                        .unwrap_or(false)))
        {
            return Err(DomainError::Unavailable(
                "accepted command queue requires settlement before replacement".into(),
            ));
        }
        let store = next.runtime.store.clone();
        let session = target.session;
        let saved = next
            .runtime
            .pool
            .submit(move || store.journal_snapshot(&session))
            .await?;
        if saved.revision != next.runtime.journal_revision()? {
            return Err(DomainError::Conflict);
        }
        let runtime = previous.services.runtime();
        let scope = {
            let state = runtime.state()?;
            ag1_core::tools::registry::Scope {
                workspace: runtime.workspace.clone(),
                session: state.stamp().session().as_str().into(),
                epoch: state.stamp().epoch(),
                revision: state.journal_revision(),
            }
        };
        let operation = format!("retire-{}-{epoch}", runtime.incarnation);
        let tools = previous.services.tools().clone();
        let retiring = runtime.clone();
        // The worker owns the old fence while accepting durable background
        // settlement. Dropping the async caller cannot reopen admission before
        // that worker has finished writing its terminal records.
        let (old, quiesced) = runtime
            .pool
            .submit(move || {
                let result = (|| {
                    if cancel_pending && let Some(commands) = commands {
                        commands.retire_pending()?;
                    }
                    retiring.lifecycle_event_sync(
                        tools.as_ref(),
                        &serde_json::json!({"type":"session_shutdown"}),
                    )?;
                    tools.quiesce(&scope, &operation)
                })();
                Ok((old, result))
            })
            .await?;
        quiesced?;
        if self.durable.is_some() {
            self.stage_selection(old, prepared, next, epoch)?;
            return self.reconcile_selection_inner().await;
        }
        let mut selected = self.selected.lock().map_err(|_| DomainError::Conflict)?;
        if selected.epoch != expected_epoch {
            return Err(DomainError::Conflict);
        }
        old.retire()?;
        let next = Selection {
            epoch,
            services: Arc::new(next),
        };
        *selected = next.clone();
        prepared.activate();
        Ok(next)
    }
}
