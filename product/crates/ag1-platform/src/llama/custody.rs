//! One process-held runtime bundle and one owned model at a time. Captured
//! plans are immutable; observation cannot manufacture an ownership handle.
use super::{
    LaunchPlan,
    lifecycle::{RuntimeController, RuntimeHost},
};
use crate::{
    cutover::EndpointLease,
    executor::BlockingPool,
    provider::http::{GenerationRuntime, HttpRequest, endpoint_origin},
};
use ag1_core::{DomainError, PortFuture};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, DomainError>;

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub model: String,
    pub plan_sha256: String,
    pub action: String,
    pub elapsed_ms: u64,
}
struct State<H: RuntimeHost> {
    controller: RuntimeController<H>,
    active: Option<String>,
    closed: bool,
    events: Vec<Event>,
    // Last field: retained until the authenticated process handle is dropped.
    _lease: EndpointLease,
}
pub struct RuntimeBundle<H: RuntimeHost> {
    state: Arc<Mutex<State<H>>>,
    plans: Arc<BTreeMap<String, LaunchPlan>>,
    origin: String,
    pool: Arc<BlockingPool>,
    load_timeout: Duration,
    progress: Arc<Mutex<Option<serde_json::Value>>>,
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
impl<H: RuntimeHost + Send + 'static> RuntimeBundle<H>
where
    H::Handle: Send,
{
    pub fn new(
        host: H,
        plans: Vec<LaunchPlan>,
        lease: EndpointLease,
        load_timeout: Duration,
    ) -> Result<Self> {
        if plans.is_empty()
            || plans.len() > 128
            || load_timeout.is_zero()
            || load_timeout > Duration::from_secs(1800)
        {
            return Err(DomainError::InvalidInput("Runtime bundle limits".into()));
        }
        let mut map = BTreeMap::new();
        let origin = super::observation::origin(&plans[0])?;
        for plan in plans {
            if plan.fingerprint()? != plan.plan_sha256
                || super::observation::origin(&plan)? != origin
                || map.contains_key(&plan.model_id)
            {
                return Err(DomainError::Conflict);
            }
            map.insert(plan.model_id.clone(), plan);
        }
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                controller: RuntimeController::new(host),
                active: None,
                closed: false,
                events: Vec::new(),
                _lease: lease,
            })),
            plans: Arc::new(map),
            origin,
            pool: Arc::new(BlockingPool::new(1, 32)?),
            load_timeout,
            progress: Arc::new(Mutex::new(None)),
        })
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }
    pub fn events(&self) -> Result<Vec<Event>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?
            .events
            .clone())
    }
    pub async fn resources(&self) -> Result<Option<serde_json::Value>> {
        let state = self.state.clone();
        self.pool
            .submit(move || {
                state
                    .lock()
                    .map_err(|_| DomainError::DurabilityUnknown)?
                    .controller
                    .owned_resources()
            })
            .await
    }
    pub async fn stop(&self) -> Result<()> {
        self.stop_current(true).await
    }
    /// Host-only cold restart/uncertain-generation settlement. The bundle lease
    /// remains held; successful return proves the authenticated child has exited.
    pub async fn unload_owned(&self) -> Result<()> {
        self.stop_current(false).await
    }
    async fn stop_current(&self, close: bool) -> Result<()> {
        let state = self.state.clone();
        let plans = self.plans.clone();
        let progress = self.progress.clone();
        self.pool
            .submit(move || {
                let mut s = state.lock().map_err(|_| DomainError::DurabilityUnknown)?;
                // Persist the admission fence in memory before attempting owned stop.
                // A failed stop retains both handle and bundle lease for recovery.
                s.closed |= close;
                let started = Instant::now();
                s.controller.stop_owned_runtime()?;
                if let Some(model) = s.active.take() {
                    s.events.push(Event {
                        plan_sha256: plans[&model].plan_sha256.clone(),
                        model,
                        action: "owned_stop".into(),
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    });
                    if s.events.len() > 1024 {
                        s.events.remove(0);
                    }
                }
                if let Ok(mut value) = progress.lock() {
                    *value = None;
                }
                Ok(())
            })
            .await
    }
    async fn settle_cancelled_generation(&self) -> Result<()> {
        let state = self.state.clone();
        let plans = self.plans.clone();
        let timeout = self.load_timeout.min(Duration::from_secs(30));
        self.pool
            .submit(move || {
                let mut s = state.lock().map_err(|_| DomainError::DurabilityUnknown)?;
                let Some(model) = s.active.clone() else {
                    return Ok(());
                };
                let plan = &plans[&model];
                let started = Instant::now();
                loop {
                    // Closing the response cancels llama.cpp's request. Retain the
                    // model and lease, and acknowledge only verified slot settlement.
                    if s.controller.poll_generation_idle(plan)? {
                        s.events.push(Event {
                            model,
                            plan_sha256: plan.plan_sha256.clone(),
                            action: "owned_generation_cancelled".into(),
                            elapsed_ms: started.elapsed().as_millis() as u64,
                        });
                        if s.events.len() > 1024 {
                            s.events.remove(0);
                        }
                        return Ok(());
                    }
                    if started.elapsed() >= timeout {
                        return Err(DomainError::TimedOut);
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                // An unavailable/uncertain observation keeps the model alive and
                // leaves the caller's generation permit quarantined for recovery.
            })
            .await
    }
    async fn ensure(&self, model: String) -> Result<()> {
        if !self.plans.contains_key(&model) {
            return Err(DomainError::PermissionDenied);
        }
        let state = self.state.clone();
        let plans = self.plans.clone();
        let timeout = self.load_timeout;
        let progress = self.progress.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(cancelled.clone());
        self.pool
            .submit(move || {
                let mut s = state.lock().map_err(|_| DomainError::DurabilityUnknown)?;
                if s.closed {
                    return Err(DomainError::PermissionDenied);
                }
                let plan = &plans[&model];
                let previous_progress = progress.lock().ok().and_then(|value| value.clone());
                let mut started_here = false;
                let started_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                let phase = |name: &str| {
                    if let Ok(mut value) = progress.lock() {
                        *value = Some(serde_json::json!({"model":model,"phase":name,"started_at":started_at}));
                    }
                };
                let started = Instant::now();
                let result = (|| {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(DomainError::Cancelled);
                    }
                    // Rehash/material identity checks run for every admission, even
                    // when an already owned model reports healthy on the endpoint.
                    phase("verifying");
                    s.controller
                        .host_mut()
                        .verify_progress(&plan.runtime_materials()?, &mut |percent| {
                            if let Ok(mut value) = progress.lock() {
                                *value = Some(serde_json::json!({"model":model,"phase":"verifying","percent":percent,"started_at":started_at}));
                            }
                        })?;
                    if cancelled.load(Ordering::Acquire) {
                        return Err(DomainError::Cancelled);
                    }
                    if let Some(previous) = s.active.clone()
                        && s.controller.reap_exited_owned_runtime()?
                    {
                        s.events.push(Event {
                            plan_sha256: plans[&previous].plan_sha256.clone(),
                            model: previous,
                            action: "owned_exit".into(),
                            elapsed_ms: started.elapsed().as_millis() as u64,
                        });
                        s.active = None;
                    }
                    if s.active.as_deref() != Some(&model) {
                        if let Some(previous) = s.active.clone() {
                            phase("releasing");
                            s.controller.stop_owned_runtime()?;
                            s.events.push(Event {
                                model: previous.clone(),
                                plan_sha256: plans[&previous].plan_sha256.clone(),
                                action: "owned_stop".into(),
                                elapsed_ms: started.elapsed().as_millis() as u64,
                            });
                            s.active = None;
                        }
                        if cancelled.load(Ordering::Acquire) {
                            return Err(DomainError::Cancelled);
                        }
                        phase("loading");
                        s.controller.start_owned_runtime(plan)?;
                        started_here = true;
                        s.active = Some(model.clone());
                        s.events.push(Event {
                            model: model.clone(),
                            plan_sha256: plan.plan_sha256.clone(),
                            action: "owned_start".into(),
                            elapsed_ms: started.elapsed().as_millis() as u64,
                        });
                    }
                    loop {
                        if cancelled.load(Ordering::Acquire) {
                            return Err(DomainError::Cancelled);
                        }
                        if started.elapsed() >= timeout {
                            return Err(DomainError::TimedOut);
                        }
                        if s.controller.poll_ready(plan)? {
                            break;
                        }
                        if let Some(name) = s.controller.loading_phase() { phase(name); }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    s.events.push(Event {
                        model: model.clone(),
                        plan_sha256: plan.plan_sha256.clone(),
                        action: "owned_ready".into(),
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    });
                    phase("ready");
                    Ok(())
                })();
                if result == Err(DomainError::Cancelled) && !started_here {
                    // Cancelling admission to an existing model must not eject
                    // it. A newly started, incomplete load still drains below.
                    if let Ok(mut value) = progress.lock() {
                        *value = if s.active.is_some() { previous_progress } else { None };
                    }
                } else if let Err(error) = &result {
                    phase("failed");
                    if let Ok(mut value) = progress.lock()
                        && let Some(value) = value.as_mut() {
                        value["error"] = serde_json::json!(format!("{error:?}"));
                    }
                    // Only the controller's authenticated handle can be stopped.
                    s.controller.stop_owned_runtime()?;
                    if let Some(model) = s.active.take() {
                        s.events.push(Event {
                            plan_sha256: plans[&model].plan_sha256.clone(),
                            model,
                            action: "owned_stop".into(),
                            elapsed_ms: started.elapsed().as_millis() as u64,
                        });
                    }
                }
                if s.events.len() > 1024 {
                    let excess = s.events.len() - 1024;
                    s.events.drain(..excess);
                }
                result
            })
            .await
    }
}
impl<H: RuntimeHost + Send + 'static> GenerationRuntime for RuntimeBundle<H>
where
    H::Handle: Send,
{
    fn progress(&self) -> Option<serde_json::Value> {
        // Never wait behind weight verification/loading on a UI polling thread.
        if let Ok(mut state) = self.state.try_lock()
            && state.active.is_some()
            && !state.controller.owned_alive().unwrap_or(false)
            && let Ok(mut value) = self.progress.lock()
        {
            *value = None;
        }
        self.progress.lock().ok().and_then(|value| value.clone())
    }
    fn load_model<'a>(&'a self, model: &'a str) -> PortFuture<'a, ()> {
        Box::pin(self.ensure(model.to_owned()))
    }
    fn cancel_generation<'a>(&'a self, url: &'a str) -> PortFuture<'a, ()> {
        Box::pin(async move {
            if endpoint_origin(url)? != self.origin {
                return Err(DomainError::PermissionDenied);
            }
            self.settle_cancelled_generation().await
        })
    }
    fn shutdown(&self) -> PortFuture<'_, ()> {
        Box::pin(self.stop())
    }
    fn prepare<'a>(&'a self, request: &'a HttpRequest) -> PortFuture<'a, ()> {
        Box::pin(async move {
            if endpoint_origin(&request.url)? != self.origin {
                return Ok(());
            }
            let value: serde_json::Value = serde_json::from_slice(&request.body)
                .map_err(|_| DomainError::InvalidInput("Local generation request".into()))?;
            let model = value["model"]
                .as_str()
                .ok_or(DomainError::PermissionDenied)?;
            self.ensure(model.to_owned()).await
        })
    }
}
