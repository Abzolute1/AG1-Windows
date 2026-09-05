//! Owned background jobs with P05 CAS records. P10 calls refresh on its timer and
//! acknowledges stable notices only after parent-journal admission. No historical PID is adopted.
use crate::shell_tools::{PreparedShell, RunningShell, ShellExecution, ShellService};
use ag1_core::{
    Clock, DomainError,
    storage::StoragePort,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, Effect, ExecutionArtifacts},
        jobs::{Book, Notice, Record, Status},
        registry::{Origin, Scope},
        shell::Exit,
    },
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub struct JobService<S: StoragePort> {
    store: Arc<S>,
    shell: Arc<ShellService>,
    clock: Arc<dyn Clock>,
    scope: Scope,
    incarnation: String,
    key: String,
    live: Mutex<BTreeMap<String, RunningShell>>,
}
impl<S: StoragePort> JobService<S> {
    /// One host-owned service per session. A fresh incarnation is required after process restart.
    pub fn new(
        store: Arc<S>,
        shell: Arc<ShellService>,
        clock: Arc<dyn Clock>,
        scope: Scope,
        incarnation: &str,
    ) -> Result<Self, DomainError> {
        ag1_core::OperationId::new(incarnation)?;
        if scope.workspace != shell.workspace().id().as_str() {
            return Err(DomainError::PermissionDenied);
        }
        let key = format!(
            "jobs-{:x}",
            Sha256::digest(json!([scope.workspace, scope.session]).to_string())
        );
        Ok(Self {
            store,
            shell,
            clock,
            scope,
            incarnation: incarnation.into(),
            key,
            live: Mutex::new(BTreeMap::new()),
        })
    }
    pub fn target_revision(&self) -> &str {
        self.shell.target_revision()
    }
    fn load(&self) -> Result<(Option<String>, Book), DomainError> {
        if let Some(document) = self.store.load_metadata("background-jobs", &self.key)? {
            if document.bytes.len() > 2 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("job records".into()));
            }
            let book: Book = serde_json::from_slice(&document.bytes)
                .map_err(|_| DomainError::InvalidInput("job records".into()))?;
            if book.version != 1
                || book.workspace != self.scope.workspace
                || book.session != self.scope.session
                || book.jobs.len() > 200
            {
                return Err(DomainError::Conflict);
            }
            Ok((Some(document.sha256), book))
        } else {
            Ok((
                None,
                Book {
                    version: 1,
                    workspace: self.scope.workspace.clone(),
                    session: self.scope.session.clone(),
                    jobs: vec![],
                },
            ))
        }
    }
    fn save(&self, expected: Option<&str>, book: &Book) -> Result<String, DomainError> {
        let bytes = serde_json::to_vec(book)
            .map_err(|_| DomainError::InvalidInput("job records".into()))?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("job records".into()));
        }
        self.store
            .commit_metadata("background-jobs", &self.key, expected, &bytes)
    }
    fn validate_scope(&self, scope: &Scope) -> Result<(), DomainError> {
        if scope.workspace != self.scope.workspace
            || scope.session != self.scope.session
            || scope.epoch != self.scope.epoch
        {
            Err(DomainError::PermissionDenied)
        } else {
            Ok(())
        }
    }
    /// Explicit restart handoff, called after the previous owner has been retired. No signals are sent.
    pub fn recover_orphans(&self) -> Result<usize, DomainError> {
        let live = self.live.lock().unwrap();
        let (revision, mut book) = self.load()?;
        let mut count = 0;
        for job in &mut book.jobs {
            if matches!(job.status, Status::Intent | Status::Running) && !live.contains_key(&job.id)
            {
                job.status = Status::Unknown;
                job.finished_at = Some(self.clock.now_ms());
                job.pid = None;
                count += 1;
            }
        }
        if count > 0 {
            self.save(revision.as_deref(), &book)?;
        }
        Ok(count)
    }
    pub fn refresh(&self, artifacts: &mut impl ExecutionArtifacts) -> Result<(), DomainError> {
        let mut live = self.live.lock().unwrap();
        self.refresh_locked(&mut live, artifacts)
    }
    fn refresh_locked(
        &self,
        live: &mut BTreeMap<String, RunningShell>,
        artifacts: &mut impl ExecutionArtifacts,
    ) -> Result<(), DomainError> {
        let (revision, mut book) = self.load()?;
        let mut completed = vec![];
        let mut changed = false;
        for job in &mut book.jobs {
            if !matches!(job.status, Status::Intent | Status::Running) {
                completed.push(job.id.clone());
                continue;
            }
            let Some(process) = live.get_mut(&job.id) else {
                continue;
            };
            if job.owner != self.incarnation || job.epoch != self.scope.epoch {
                return Err(DomainError::Conflict);
            }
            if let Some(exit) = process.poll(&AtomicBool::new(false))? {
                let outcome = process.capture(exit, &job.call_id, "job", artifacts)?;
                let result = artifacts.put_evidence(
                    &outcome.result.to_string(),
                    "job-result",
                    &json!({"type":"tool","name":"job","callId":job.call_id}),
                    &[],
                    None,
                )?;
                job.result = Some(result);
                job.status = Status::Exited;
                job.exit_code = match exit {
                    Exit::Code(code) => Some(code),
                    _ => None,
                };
                job.finished_at = Some(self.clock.now_ms());
                completed.push(job.id.clone());
                changed = true;
            } else if job.status == Status::Intent {
                job.status = Status::Running;
                job.pid = Some(process.child.identity().pid);
                changed = true;
            }
        }
        if changed {
            self.save(revision.as_deref(), &book)?;
        }
        for id in completed {
            live.remove(&id);
        }
        Ok(())
    }
    pub fn notices(&self) -> Result<Vec<Notice>, DomainError> {
        let (_, book) = self.load()?;
        Ok(book
            .jobs
            .iter()
            .filter(|j| matches!(j.status, Status::Exited | Status::Unknown) && !j.notified)
            .map(|job| Notice {
                id: job.notice_id(),
                text: format!(
                    "Background jobs finished:\n- {}\nUse job poll/log for output.",
                    job.line(self.clock.now_ms())
                ),
            })
            .collect())
    }
    /// Called after P10 has durably appended this stable notice ID, never on mere delivery.
    pub fn acknowledge_notice(&self, id: &str) -> Result<(), DomainError> {
        let _live = self.live.lock().unwrap();
        let (revision, mut book) = self.load()?;
        let job = book
            .jobs
            .iter_mut()
            .find(|j| j.notice_id() == id)
            .ok_or(DomainError::NotFound)?;
        if !matches!(job.status, Status::Exited | Status::Unknown) {
            return Err(DomainError::Conflict);
        }
        if !job.notified {
            job.notified = true;
            self.save(revision.as_deref(), &book)?;
        }
        Ok(())
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        plan: Option<PreparedShell>,
        authorization: AuthorizedCall,
        execution: ShellExecution<'_, F>,
        artifacts: &mut impl ExecutionArtifacts,
    ) -> Result<AdapterOutcome, DomainError> {
        let ShellExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        self.validate_scope(scope)?;
        if authorization.call().origin() != Origin::BuiltIn || authorization.call().name() != "job"
        {
            return Err(DomainError::PermissionDenied);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(DomainError::TimedOut);
        }
        let call = authorization.consume(
            registry,
            ExecutionFence {
                scope,
                policy_revision,
                target_revision: self.target_revision(),
                now: now(),
            },
        )?;
        let action = call.args()["action"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("job action".into()))?;
        let mut live = self.live.lock().unwrap();
        self.refresh_locked(&mut live, artifacts)?;
        let (revision, mut book) = self.load()?;
        let now = self.clock.now_ms();
        let mut effect = Effect::None;
        let result = match action {
            "start" => {
                let plan = plan.ok_or_else(|| {
                    DomainError::InvalidInput("prepared job command required".into())
                })?;
                if plan.call().operation() != call.operation()
                    || plan.call().args_hash() != call.args_hash()
                    || plan.target_revision() != self.target_revision()
                {
                    return Err(DomainError::Conflict);
                }
                if plan.command.trim().is_empty() {
                    return Err(DomainError::InvalidInput("start requires command.".into()));
                }
                if live.len() >= 20
                    || book
                        .jobs
                        .iter()
                        .filter(|j| matches!(j.status, Status::Running | Status::Intent))
                        .count()
                        >= 20
                {
                    return Err(DomainError::QueueFull);
                }
                if book.jobs.len() >= 200 {
                    let remove = book
                        .jobs
                        .iter()
                        .position(|j| {
                            matches!(j.status, Status::Exited | Status::Unknown) && j.notified
                        })
                        .ok_or(DomainError::QueueFull)?;
                    // P05 retains the previous metadata version; full output artifacts remain separately owned.
                    book.jobs.remove(remove);
                }
                let id = format!(
                    "job-{}",
                    &format!(
                        "{:x}",
                        Sha256::digest(
                            json!([scope.workspace, scope.session, call.operation()]).to_string()
                        )
                    )[..24]
                );
                if book.jobs.iter().any(|j| j.id == id) {
                    return Err(DomainError::Conflict);
                }
                let job = Record {
                    id: id.clone(),
                    operation: call.operation().into(),
                    call_id: call.call_id().into(),
                    command: plan.command.clone(),
                    owner: self.incarnation.clone(),
                    epoch: scope.epoch,
                    started_at: now,
                    status: Status::Intent,
                    pid: None,
                    exit_code: None,
                    finished_at: None,
                    result: None,
                    notified: false,
                };
                book.jobs.push(job);
                let revision = self.save(revision.as_deref(), &book)?;
                // One acknowledged durable intent precedes the only process launch.
                let launch = if cancel.load(Ordering::Acquire) {
                    Err(DomainError::Cancelled)
                } else if Instant::now() >= deadline {
                    Err(DomainError::TimedOut)
                } else {
                    self.shell.start(
                        &plan.command,
                        86400.,
                        Instant::now() + Duration::from_secs(86400),
                    )
                };
                let process = match launch {
                    Ok(process) => process,
                    Err(error) => {
                        let job = book.jobs.last_mut().unwrap();
                        job.status = Status::Unknown;
                        job.finished_at = Some(self.clock.now_ms());
                        self.save(Some(&revision), &book)?;
                        return Err(error);
                    }
                };
                let pid = process.child.identity().pid;
                live.insert(id.clone(), process);
                let job = book.jobs.last_mut().unwrap();
                job.pid = Some(pid);
                job.status = Status::Running;
                let line = job.line(now);
                self.save(Some(&revision), &book)?;
                effect = Effect::Committed;
                json!({"content":[{"type":"text","text":format!("Started {id} (pid {pid}). Keep working; you will be told when it finishes.\n{line}")}],"details":{"action":"start","id":id,"status":"running"}})
            }
            "list" => {
                let text = if book.jobs.is_empty() {
                    "No background jobs.".into()
                } else {
                    book.jobs
                        .iter()
                        .rev()
                        .take(20)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .map(|j| j.line(now))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                json!({"content":[{"type":"text","text":text}],"details":{"action":"list"}})
            }
            "poll" | "log" | "kill" => {
                let id = call.args()["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| DomainError::InvalidInput(format!("{action} requires id.")))?;
                let job = book
                    .jobs
                    .iter_mut()
                    .find(|j| j.id == id)
                    .ok_or_else(|| DomainError::InvalidInput(format!("No job {id}.")))?;
                if action == "kill" {
                    if job.status != Status::Running
                        || job.owner != self.incarnation
                        || job.epoch != scope.epoch
                    {
                        return Err(DomainError::InvalidInput(format!(
                            "Job {id} is not running."
                        )));
                    }
                    let process = live.get_mut(id).ok_or(DomainError::PermissionDenied)?;
                    process.child.stop()?;
                    let outcome =
                        process.capture(Exit::Cancelled, &job.call_id, "job", artifacts)?;
                    job.result = Some(artifacts.put_evidence(
                        &outcome.result.to_string(),
                        "job-result",
                        &json!({"type":"tool","name":"job","callId":job.call_id}),
                        &[],
                        None,
                    )?);
                    job.status = Status::Exited;
                    job.exit_code = None;
                    job.finished_at = Some(now);
                    self.save(revision.as_deref(), &book)?;
                    live.remove(id);
                    effect = Effect::Committed;
                    json!({"content":[{"type":"text","text":format!("Killed {id}.")}],"details":{"action":"kill","id":id,"status":"exited"}})
                } else {
                    let mut text = job.line(now);
                    if action == "log" || job.status != Status::Running {
                        let output = if let Some(result) = &job.result {
                            let value = artifacts.read_json(result)?;
                            if value["details"]["termination"] == "exit" {
                                value["details"]["logTail"]
                                    .as_str()
                                    .ok_or(DomainError::Conflict)?
                                    .to_owned()
                            } else {
                                value["content"][0]["text"]
                                    .as_str()
                                    .ok_or(DomainError::Conflict)?
                                    .to_owned()
                            }
                        } else if let Some(process) = live.get(id) {
                            ag1_core::tools::shell::sanitize(&process.child.tail(64 * 1024)?.bytes)?
                        } else {
                            "(no output yet)".into()
                        };
                        let tail = output
                            .rsplit('\n')
                            .take(40)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect::<Vec<_>>()
                            .join("\n");
                        if action == "log" || !tail.trim().is_empty() {
                            text.push_str(&format!("\n\n{tail}"));
                        }
                    }
                    json!({"content":[{"type":"text","text":text}],"details":{"action":action,"id":id,"status":job.status,"resultArtifact":job.result}})
                }
            }
            _ => return Err(DomainError::InvalidInput("unsupported job action".into())),
        };
        Ok(AdapterOutcome {
            result,
            effect,
            artifact_page: false,
        })
    }
}
