//! Retain both admission fences until the selected-owner transaction is
//! acknowledged. A dropped caller never decides whether a SQL write committed.
use super::*;
use ag1_storage::{RuntimeSelection, SelectionChange, StoredSelection};
use std::sync::{
    MutexGuard,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Durability {
    state: Mutex<DurableState>,
}
pub(super) struct DurableState {
    revision: Option<String>,
    pub(super) pending: Option<Arc<Pending>>,
}
impl Durability {
    pub(super) fn lock(&self) -> Result<MutexGuard<'_, DurableState>, DomainError> {
        self.state.lock().map_err(|_| DomainError::Conflict)
    }
}
pub(super) struct Pending {
    change: SelectionChange,
    next: Selection,
    fences: Mutex<Option<Fences>>,
    writing: AtomicBool,
}
struct Fences {
    old: Option<Frozen>,
    prepared: Option<PreparedFence>,
}
impl Drop for Fences {
    fn drop(&mut self) {
        // A registry dropped during an ambiguous write leaves both owners
        // unavailable. Accepted SQL owns an Arc<Pending> until it has drained.
        if let Some(old) = self.old.take() {
            let _ = old.retire();
        }
    }
}
struct Writing(Arc<Pending>);
impl Drop for Writing {
    fn drop(&mut self) {
        self.0.writing.store(false, Ordering::Release);
    }
}
fn change(
    next: &Selection,
    expected: Option<String>,
    command: String,
) -> Result<SelectionChange, DomainError> {
    let runtime = next.services.runtime();
    let (session, journal_revision) = {
        let state = runtime.state()?;
        (
            state.stamp().session().as_str().into(),
            state.journal_revision(),
        )
    };
    Ok(SelectionChange {
        expected,
        selection: RuntimeSelection {
            version: 1,
            workspace: runtime.workspace.clone(),
            session,
            epoch: next.epoch,
            owner: runtime.incarnation.clone(),
            journal_revision,
            command,
        },
    })
}
impl RuntimeRegistry {
    /// Stage a fresh native owner, initially unavailable. The host must have
    /// drained any previous live registry before constructing its successor.
    /// On restart, prepare the session named by the current stored selection;
    /// no persisted process, job or incomplete effect is adopted or replayed.
    /// Call reconcile_selection to acknowledge this fresh owner before use.
    pub fn durable(
        initial: PreparedRuntime,
        previous: Option<StoredSelection>,
        command: String,
    ) -> Result<Self, DomainError> {
        let frozen = initial.runtime.freeze()?;
        if !frozen.idle()?
            || !initial.runtime.settled()?
            || !initial.runtime.persistence_healthy()?
        {
            return Err(DomainError::Conflict);
        }
        let prepared = PreparedFence(Some(frozen));
        if let Some(previous) = &previous
            && (previous.selection.workspace != initial.runtime.workspace
                || previous.selection.session
                    != initial.runtime.state()?.stamp().session().as_str()
                || previous.selection.owner == initial.runtime.incarnation)
        {
            return Err(DomainError::Conflict);
        }
        let epoch = previous
            .as_ref()
            .map_or(Some(1), |p| p.selection.epoch.checked_add(1))
            .ok_or(DomainError::Conflict)?;
        let revision = previous.map(|p| p.sha256);
        let next = Selection {
            epoch,
            services: Arc::new(initial),
        };
        let pending = Pending {
            change: change(&next, revision.clone(), command)?,
            next: next.clone(),
            fences: Mutex::new(Some(Fences {
                old: None,
                prepared: Some(prepared),
            })),
            writing: AtomicBool::new(false),
        };
        Ok(Self {
            selected: Mutex::new(next),
            replacing: tokio::sync::Mutex::new(()),
            durable: Some(Durability {
                state: Mutex::new(DurableState {
                    revision,
                    pending: Some(Arc::new(pending)),
                }),
            }),
        })
    }

    /// Exact pending transaction for host recovery/display, never admission.
    pub fn pending_selection(&self) -> Result<Option<SelectionChange>, DomainError> {
        self.durable
            .as_ref()
            .map(|d| Ok(d.lock()?.pending.as_ref().map(|p| p.change.clone())))
            .transpose()
            .map(Option::flatten)
    }

    pub(super) fn stage_selection(
        &self,
        old: Frozen,
        prepared: PreparedFence,
        next: PreparedRuntime,
        epoch: u64,
    ) -> Result<(), DomainError> {
        let mut state = self.durable.as_ref().ok_or(DomainError::Conflict)?.lock()?;
        if state.pending.is_some() {
            return Err(DomainError::Conflict);
        }
        let command = format!("select-{}-{epoch}", next.runtime.incarnation);
        let next = Selection {
            epoch,
            services: Arc::new(next),
        };
        let change = change(&next, state.revision.clone(), command)?;
        state.pending = Some(Arc::new(Pending {
            change,
            next,
            fences: Mutex::new(Some(Fences {
                old: Some(old),
                prepared: Some(prepared),
            })),
            writing: AtomicBool::new(false),
        }));
        Ok(())
    }

    /// Retry only the selected-owner SQL transaction. Both runtimes remain
    /// frozen after an error or dropped awaiter; this never repeats execution.
    pub async fn reconcile_selection(&self) -> Result<Selection, DomainError> {
        let _replacement = self
            .replacing
            .try_lock()
            .map_err(|_| DomainError::Conflict)?;
        self.reconcile_selection_inner().await
    }

    pub(super) async fn reconcile_selection_inner(&self) -> Result<Selection, DomainError> {
        let durable = self.durable.as_ref().ok_or(DomainError::Conflict)?;
        let pending = durable.lock()?.pending.clone();
        let Some(pending) = pending else {
            return self.selected();
        };
        pending
            .writing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| DomainError::Conflict)?;
        let writing = Writing(pending.clone());
        let store = pending.next.services.runtime.store.clone();
        let ack = pending
            .next
            .services
            .runtime
            .pool
            .submit(move || {
                let ack = store.commit_runtime_selection(&writing.0.change)?;
                let current = store.runtime_selection(&ack.selection.workspace)?;
                if current.as_ref().is_none_or(|record| {
                    record.sha256 != ack.sha256 || record.selection != ack.selection
                }) {
                    return Err(DomainError::Conflict);
                }
                Ok(ack)
            })
            .await?;
        let mut state = durable.lock()?;
        let mut selected = self.selected.lock().map_err(|_| DomainError::Conflict)?;
        if state
            .pending
            .as_ref()
            .is_none_or(|current| !Arc::ptr_eq(current, &pending))
        {
            return Err(DomainError::Conflict);
        }
        let mut fences = pending.fences.lock().map_err(|_| DomainError::Conflict)?;
        let mut fences = fences.take().ok_or(DomainError::Conflict)?;
        if let Some(old) = fences.old.take() {
            old.retire()?;
        }
        let prepared = fences.prepared.take().ok_or(DomainError::Conflict)?;
        *selected = pending.next.clone();
        state.revision = Some(ack.sha256);
        prepared.activate();
        state.pending = None;
        Ok(selected.clone())
    }
}
