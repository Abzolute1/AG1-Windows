use super::*;
use ag1_core::runtime::planning::Status;

pub struct Execution {
    pub plan: PlanRef,
    pub step_index: Option<usize>,
}
/// Execute an explicitly selected accepted plan under the host's normal tool
/// policy. Workflow acceptance is not a substitute for per-tool authorization.
/// Interrupted work retains its executing record for inspection, never replay.
pub async fn execute(
    host: Arc<NativeSession>,
    input: TurnInput,
    resolver: Arc<dyn ProviderResolver>,
    runner: Arc<dyn ToolRunner>,
    listener: Arc<dyn Listener>,
    execution: Execution,
) -> Result<PlanningResult, DomainError> {
    let activity = host.enter_owned()?;
    if host.commands_claimed.load(Ordering::Acquire) || !host.settled()? {
        return Err(DomainError::Conflict);
    }
    ag1_core::OperationId::new(&input.command)?;
    if input.max_generations == Some(0) {
        return Err(DomainError::InvalidInput("execution budget".into()));
    }
    let driver = tokio::spawn(async move {
        let _activity = activity;
        execute_selected(
            host.as_ref(),
            input,
            resolver.as_ref(),
            runner,
            listener.as_ref(),
            (execution, super::super::RunOrigin::User),
        )
        .await
    });
    driver.await.map_err(|_| DomainError::DurabilityUnknown)?
}

pub(in crate::session) async fn execute_selected(
    host: &NativeSession,
    mut input: TurnInput,
    resolver: &dyn ProviderResolver,
    runner: Arc<dyn ToolRunner>,
    listener: &dyn Listener,
    mode: (Execution, super::super::RunOrigin),
) -> Result<PlanningResult, DomainError> {
    let (execution, origin) = mode;
    ag1_core::OperationId::new(&input.command)?;
    if input.max_generations == Some(0) {
        return Err(DomainError::InvalidInput("execution budget".into()));
    }
    let lease = ModeLease::begin(host, None)?;
    let command = format!(
        "execute-plan-{}",
        ag1_storage::digest(input.command.as_bytes())
    );
    let timestamp = input.timestamp.clone();
    let (action, plan, index) = {
        let mut state = host.state()?;
        let plan =
            planning::resolve(state.branch(), &execution.plan).ok_or(DomainError::NotFound)?;
        let mut selection = planning::selection(state.branch()).ok_or(DomainError::NotFound)?;
        if selection.accepted.as_ref() != Some(&execution.plan)
            || !matches!(plan.status, Status::Accepted | Status::Executing)
        {
            return Err(DomainError::Conflict);
        }
        let index = execution
            .step_index
            .or_else(|| plan.next_step())
            .ok_or(DomainError::NotFound)?;
        let plan = plan.begin_step(index, &timestamp)?;
        selection.planning = None;
        selection.executing = Some(plan.reference());
        selection.updated_at = timestamp.clone();
        input.message = serde_json::json!({"role":"user","content":plan.execution_prompt(execution.step_index)?,"timestamp":chrono::DateTime::parse_from_rfc3339(&timestamp).map_err(|_|DomainError::InvalidInput("execution timestamp".into()))?.timestamp_millis()});
        let action = state.update_plan(
            &command,
            &command,
            &timestamp,
            Some(plan.clone()),
            selection,
        )?;
        (action, plan, index)
    };
    host.commit(action, listener).await?;
    let turn = host
        .run_inner(input, resolver, runner, listener, origin, Some(&lease))
        .await?;
    if lease.cancelled.load(Ordering::Acquire) {
        return Err(DomainError::Cancelled);
    }
    let text = ag1_core::context::hygiene::content(&turn.assistant, "\n");
    let reviewed = plan.record_review(index, &text, &timestamp)?;
    if let Some(reviewed) = &reviewed {
        host.wait_for_commit().await?;
        let action = {
            let mut state = host.state()?;
            let mut selection = planning::selection(state.branch()).ok_or(DomainError::NotFound)?;
            if selection.executing.as_ref() != Some(&plan.reference()) {
                return Err(DomainError::Conflict);
            }
            if reviewed.status != Status::Executing {
                selection.executing = None;
            }
            selection.updated_at = timestamp.clone();
            let command = format!("{command}-review");
            state.update_plan(
                &command,
                &command,
                &timestamp,
                Some(reviewed.clone()),
                selection,
            )?
        };
        host.commit(action, listener).await?;
    }
    Ok(PlanningResult {
        turn,
        plan: Some(reviewed.unwrap_or(plan)),
    })
}
