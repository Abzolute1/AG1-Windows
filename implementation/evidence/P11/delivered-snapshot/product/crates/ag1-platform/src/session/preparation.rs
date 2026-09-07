//! Cancellation of an unsent request releases only its exact core reservation.
//! Accepted blocking work has its own retained worker lifetime and still drains.
use ag1_core::{
    DomainError,
    runtime::{SessionRuntime, session::PreparationAction},
};
use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

pub(super) struct RequestPreparation {
    action: Option<PreparationAction>,
    state: Arc<Mutex<SessionRuntime>>,
    changed: Arc<tokio::sync::Notify>,
}
impl RequestPreparation {
    pub(super) fn new(
        action: PreparationAction,
        state: Arc<Mutex<SessionRuntime>>,
        changed: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            action: Some(action),
            state,
            changed,
        }
    }
    pub(super) fn into_action(mut self) -> PreparationAction {
        self.action.take().expect("owned request preparation")
    }
}
impl Deref for RequestPreparation {
    type Target = PreparationAction;
    fn deref(&self) -> &Self::Target {
        self.action.as_ref().expect("owned request preparation")
    }
}
impl DerefMut for RequestPreparation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.action.as_mut().expect("owned request preparation")
    }
}
impl Drop for RequestPreparation {
    fn drop(&mut self) {
        if let Some(action) = self.action.take() {
            if let Ok(mut state) = self.state.lock() {
                let _ = state.admit_request(action, Err(DomainError::Cancelled));
            }
            self.changed.notify_waiters();
        }
    }
}
