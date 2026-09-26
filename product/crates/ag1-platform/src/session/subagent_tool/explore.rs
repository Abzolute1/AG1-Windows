//! The legacy explore facade uses native read-only children and the parent's
//! scheduler. Its board belongs to the new store, never the original worktree.
use super::*;
use ag1_core::{runtime::subagent::explore as board, storage::StoragePort};

pub struct PreparedExplore {
    call: ValidatedCall,
    scope: Scope,
    target: String,
    board_hash: String,
    questions: Vec<String>,
    prior: String,
    journal: Arc<ToolJournal>,
}
impl PreparedExplore {
    pub fn target_revision(&self) -> &str {
        &self.target
    }
}

fn current(parent: &NativeSession) -> Result<(Value, Option<String>, String), DomainError> {
    if let Some(document) = parent
        .store
        .load_metadata("explore-board", &parent.workspace)?
    {
        let value = serde_json::from_slice(&document.bytes).map_err(|_| DomainError::Conflict)?;
        return Ok((
            board::board(&value)?,
            Some(document.sha256.clone()),
            document.sha256,
        ));
    }
    let (value, hash) = match parent
        .root
        .snapshot(".ag1/explore-board.json", 4 * 1024 * 1024)
    {
        Ok(snapshot) => (
            serde_json::from_slice(&snapshot.bytes).map_err(|_| DomainError::Conflict)?,
            ag1_storage::digest(&snapshot.bytes),
        ),
        Err(DomainError::NotFound) => {
            let v = board::empty();
            let hash = ag1_storage::digest(v.to_string().as_bytes());
            (v, hash)
        }
        Err(error) => return Err(error),
    };
    Ok((board::board(&value)?, None, hash))
}

impl DelegationService {
    pub fn prepare_explore(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
        journal: Option<&Arc<ToolJournal>>,
    ) -> Result<PreparedExplore, DomainError> {
        let parent = self.parent.upgrade().ok_or(DomainError::Cancelled)?;
        if call.name() != "explore"
            || call.origin() != Origin::BuiltIn
            || scope.workspace != parent.workspace
            || scope.session != parent.state()?.stamp().session().as_str()
        {
            return Err(DomainError::PermissionDenied);
        }
        let questions = board::questions(call.args())?;
        domain::routing::select(
            Profile::Explore,
            None,
            &parent.configuration()?.model,
            &self.models,
        )?;
        let journal = journal.ok_or(DomainError::PermissionDenied)?.clone();
        journal.snapshot(scope)?;
        let (value, _, board_hash) = current(&parent)?;
        Ok(PreparedExplore {
            call: call.clone(),
            scope: scope.clone(),
            target: format!(
                "explore:{}:{}:{}:{}",
                parent.incarnation, scope.epoch, scope.revision, board_hash
            ),
            board_hash,
            questions,
            prior: {
                let mut cache: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
                let mut remaining = 32 * 1024 * 1024usize;
                board::prior_with_freshness(&value, self.template.clock.now_ms(), |reference| {
                    cache.entry(reference.to_owned()).or_insert_with(|| {
                        let Ok(source) = ag1_core::evidence::SourceReference::parse(reference) else { return "invalid".into(); };
                        if remaining == 0 { return "unverified (inspection budget)".into(); }
                        match parent.root.snapshot(source.path(), remaining.min(8 * 1024 * 1024)) {
                            Ok(snapshot) => { remaining = remaining.saturating_sub(snapshot.bytes.len()); if snapshot.revision.sha256 == source.digest() { "matching recorded revision".into() } else { "stale".into() } },
                            Err(DomainError::NotFound) => "missing".into(),
                            Err(_) => "unverified".into(),
                        }
                    }).clone()
                })?
            },
            journal,
        })
    }

    pub fn execute_explore<F: Fn() -> u64>(
        &self,
        plan: PreparedExplore,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
        artifacts: &mut impl ContextArtifactPort,
    ) -> Result<AdapterOutcome, DomainError> {
        let FileExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        if scope != &plan.scope
            || authorization.call().args_hash() != plan.call.args_hash()
            || authorization.call().operation() != plan.call.operation()
        {
            return Err(DomainError::Conflict);
        }
        let parent = self.parent.upgrade().ok_or(DomainError::Cancelled)?;
        if current(&parent)?.2 != plan.board_hash {
            return Err(DomainError::Conflict);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(DomainError::TimedOut);
        }
        plan.journal.snapshot(scope)?;
        let call = authorization.consume(
            registry,
            ExecutionFence {
                scope,
                policy_revision,
                target_revision: &plan.target,
                now: now(),
            },
        )?;
        let selected_model = domain::routing::select(Profile::Explore, None, &parent.configuration()?.model, &self.models)?;
        let endpoint = selected_model["baseUrl"].as_str().unwrap_or(&plan.journal.endpoint);
        let capacity = parent.scheduler.capacity(endpoint).unwrap_or(1);
        let results = self.runtime.block_on(async {
            let mut children = tokio::task::JoinSet::new();
            let mut controls = Vec::new();
            let mut failure = None;
            let mut results: Vec<Option<Value>> = vec![None; plan.questions.len()];
            let mut next = 0;
            let mut next_progress = Instant::now();
            while !children.is_empty() || next < plan.questions.len() {
                let stopping = failure.is_some() || cancel.load(Ordering::Acquire) || Instant::now() >= deadline;
                if stopping {
                    for control in &controls { super::super::subagent::Cancellation::cancel(control); }
                    for result in &mut results[next..] {
                        *result = Some(json!({"type":"explore","state":"cancelled","summary":"Cancelled before a worker slot became available","evidence":[]}));
                    }
                    next = plan.questions.len();
                } else {
                    while children.len() < capacity && next < plan.questions.len() {
                        let index = next;
                        next += 1;
                        let question = &plan.questions[index];
                        let prior = if plan.prior == "(empty)" {"No prior findings yet.".to_owned()} else {format!("Historical workspace-board summaries (not independently verified; recheck current source evidence before relying on them):\n{}",plan.prior)};
                        let request = Request {operation:format!("explore-{}-{index}",ag1_storage::digest(call.operation().as_bytes())),description:format!("explore {}",index+1),profile:Profile::Explore,
                            prompt:format!("Answer this exploration question with file:line evidence. Do not modify files.\n\nQuestion: {question}\n\n{prior}"),model:None,max_turns:Some(6),timeout:Some(180.),token_budget:None,resume_from:None};
                        match parent.start_subagent(request,Services {tools:self.template.clone(),provider:self.provider.clone(),models:self.models.clone()}) {
                            Ok(child) => { controls.push(child.cancellation()); children.spawn(async move {(index,child.finish().await)}); }
                            Err(error) => { results[index] = Some(json!({"type":"explore","state":"failed","summary":format!("Worker could not start: {error:?}"),"evidence":[]})); }
                        }
                    }
                }
                if children.is_empty() { continue; }
                tokio::select! {
                    result = children.join_next() => match result {
                        Some(Ok((index,Ok(value)))) if value.settled => results[index] = Some(json!(value)),
                        Some(Ok((index,Ok(value)))) => { failure.get_or_insert(DomainError::Unavailable(format!("Explore child {index} {} requires settlement ({:?}, {:?})",value.child_session,value.state,value.error))); }
                        Some(Ok((_,Err(error)))) => { failure.get_or_insert(error); }
                        _ => { failure.get_or_insert(DomainError::Unavailable("Explore child requires settlement".into())); }
                    },
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        if let Err(error) = plan.journal.service_controls() { failure.get_or_insert(error); }
                        if Instant::now() >= next_progress {
                            if let Some(control) = controls.last() {
                                if let Ok(mut progress) = control.progress() {
                                    progress.last_action = format!("{} queued, {} running, {} settled", plan.questions.len()-next, children.len(), results.iter().filter(|r|r.is_some()).count());
                                    plan.journal.delegation_progress(&call, &progress);
                                }
                            }
                            next_progress = Instant::now() + Duration::from_millis(250);
                        }
                    }
                }
            }
            if let Some(error) = failure { return Err(error); }
            results.into_iter().map(|r|r.ok_or(DomainError::Conflict)).collect::<Result<Vec<_>,_>>()
        })?;
        let at = now();
        let mut findings = board::merge(&plan.questions, &results, at)?;
        let completed = findings.iter().filter(|finding| finding["state"] == "completed").count();
        let cancelled = cancel.load(Ordering::Acquire);
        // Stable bounded identities, without per-run IDs, timing or labels.
        // Full errors remain in the retained artifact, not duplicated in context.
        let recovery_failures = results.iter().map(|result| {
            let error = if result["error"].is_null() { &result["summary"] } else { &result["error"] };
            json!({"state":result["state"],"model":result["model"],
                "errorDigest":ag1_storage::digest(error.to_string().as_bytes())})
        }).collect::<Vec<_>>();
        let raw = json!({"at":at,"findings":findings,"results":results});
        let retained = artifacts.put_evidence(
            &raw.to_string(),
            "explore-results",
            &json!({"type":"tool","name":"explore","callId":call.call_id()}),
            &[],
            None,
        )?;
        for finding in &mut findings { finding["resultArtifact"] = json!(retained.id); }
        let (existing, expected, _) = current(&parent)?;
        let next = board::append(&existing, &findings, at)?;
        parent.store.save_metadata(
            "explore-board",
            &parent.workspace,
            expected.as_deref(),
            next.to_string().as_bytes(),
        )?;
        let mut current_scope = scope.clone();
        current_scope.revision = plan.journal.revision()?;
        plan.journal
            .commit_state(&current_scope, &call, "ag1.explore_tasks", raw)
            .map_err(DomainError::from)?;
        Ok(AdapterOutcome {
            effect: Effect::Committed,
            result: json!({"content":[{"type":"text","text":board::report(&findings)?}],"isError":completed == 0 || cancelled,
            "details":{"count":findings.len(),"completed":completed,"failed":findings.len()-completed,"status":if cancelled {"cancelled"} else if completed == 0 {"failed"} else if completed < findings.len() {"partial"} else {"completed"},"states":findings.iter().map(|f|f["state"].clone()).collect::<Vec<_>>(),"recoveryFailures":recovery_failures,"resultArtifact":retained}}),
            artifact_page: false,
        })
    }
}
