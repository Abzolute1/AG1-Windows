use super::{Listener, NativeSession};
use super::{ProviderResolver, ToolRunner, TurnInput, TurnResult, print::ModeLease};
use ag1_core::runtime::{
    planning::{self, PlanRef, Submission},
    session::PreparationAction,
};
use ag1_core::{
    DomainError,
    runtime::planning::{Plan, Selection},
};
use serde_json::Value;
use std::sync::{Arc, atomic::Ordering};
mod execution;
pub(super) use execution::execute_selected;
pub use execution::{Execution, execute};

/// Host decisions for a planning turn. A saved plan or model-generated text
/// cannot turn on execution or select another inference endpoint.
pub struct Options {
    pub goal: Option<String>,
    pub previous: Option<PlanRef>,
    pub feedback: Option<String>,
    pub hidden_context: Option<String>,
    pub allow_inspection: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            goal: None,
            previous: None,
            feedback: None,
            hidden_context: None,
            allow_inspection: true,
        }
    }
}
pub struct PlanningResult {
    pub turn: TurnResult,
    pub plan: Option<Plan>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSnapshot {
    pub session: String,
    pub context_epoch: u64,
    pub journal_revision: u64,
    pub selection: Option<Selection>,
    pub plans: Vec<Plan>,
    pub more: bool,
}
pub(super) struct Turn {
    submit_tool: Value,
    addendum: String,
}
impl Turn {
    fn visible(name: &str) -> bool {
        ![
            "write",
            "edit",
            "append",
            "editApply",
            "notebook_edit",
            "submit_plan",
        ]
        .contains(&name)
    }
    pub(super) fn names(&self, names: &[String]) -> Vec<String> {
        names
            .iter()
            .filter(|s| Self::visible(s))
            .cloned()
            .chain(std::iter::once("submit_plan".into()))
            .collect()
    }
    pub(super) fn prepare(&self, preparation: &mut PreparationAction) -> Result<(), DomainError> {
        let configuration = self.configuration(preparation.configuration())?;
        preparation.set_tools(configuration.tools)?;
        preparation.set_system_prompt(configuration.system_prompt)
    }
    pub(super) fn configuration(
        &self,
        base: &ag1_core::runtime::session::Configuration,
    ) -> Result<ag1_core::runtime::session::Configuration, DomainError> {
        let tools = base
            .tools
            .iter()
            .filter(|t| t["name"].as_str().is_some_and(Self::visible))
            .cloned()
            .chain(std::iter::once(self.submit_tool.clone()))
            .collect();
        let mut configuration = base.with_tools(tools)?;
        configuration.system_prompt =
            format!("{}\n\n{}", configuration.system_prompt, self.addendum);
        configuration.validate()?;
        Ok(configuration)
    }
}

/// Accepted planning work retains its owner through generation, tool drain and
/// publication, including when the caller drops its response future. Explicit
/// NativeSession::cancel remains available throughout. No desktop/TUI owns state.
pub async fn run(
    host: Arc<NativeSession>,
    input: TurnInput,
    resolver: Arc<dyn ProviderResolver>,
    runner: Arc<dyn ToolRunner>,
    listener: Arc<dyn Listener>,
    options: Options,
) -> Result<PlanningResult, DomainError> {
    let activity = host.enter_owned()?;
    if host.commands_claimed.load(Ordering::Acquire) || !host.settled()? {
        return Err(DomainError::Conflict);
    }
    let driver = tokio::spawn(async move {
        let _activity = activity;
        run_selected(
            host.as_ref(),
            input,
            resolver.as_ref(),
            runner,
            listener.as_ref(),
            (options, super::RunOrigin::User),
        )
        .await
    });
    driver.await.map_err(|_| DomainError::DurabilityUnknown)?
}

pub(super) async fn run_selected(
    host: &NativeSession,
    input: TurnInput,
    resolver: &dyn ProviderResolver,
    runner: Arc<dyn ToolRunner>,
    listener: &dyn Listener,
    mode: (Options, super::RunOrigin),
) -> Result<PlanningResult, DomainError> {
    let (options, origin) = mode;
    let mut lease = ModeLease::begin(host, None)?;
    let goal = options
        .goal
        .as_deref()
        .map(str::to_owned)
        .unwrap_or_else(|| ag1_core::context::hygiene::content(&input.message, "\n"));
    let bytes = [
        Some(goal.as_str()),
        options.feedback.as_deref(),
        options.hidden_context.as_deref(),
    ]
    .into_iter()
    .flatten()
    .fold(0usize, |n, s| n.saturating_add(s.len()));
    if bytes > 2 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("planning guidance".into()));
    }
    let previous = options
        .previous
        .as_ref()
        .map(|reference| {
            let state = host.state()?;
            let latest = planning::plans(state.branch())
                .into_iter()
                .filter(|p| p.plan_id == reference.plan_id)
                .max_by_key(|p| p.revision)
                .ok_or(DomainError::NotFound)?;
            if latest.reference() != *reference {
                return Err(DomainError::Conflict);
            }
            Ok(latest)
        })
        .transpose()?;
    let goal = if options.goal.is_none() {
        previous.as_ref().map(|p| p.goal.clone()).unwrap_or(goal)
    } else {
        goal
    };
    let hidden = options.hidden_context.or_else(|| {
        previous.as_ref().map(|plan| {
            if let Some(feedback) = &options.feedback {
                plan.revision_prompt(feedback)
            } else {
                plan.clarification_prompt(&ag1_core::context::hygiene::content(
                    &input.message,
                    "\n",
                ))
            }
        })
    });
    let inspect = options.allow_inspection && planning::allow_inspection(&goal, previous.is_some());
    let (runner, submit_tool) = runner.planning_tools(inspect)?;
    let mut addendum = format!(
        "<planning-mode-guidance>\n{}\n</planning-mode-guidance>",
        include_str!("../../../ag1-core/resources/runtime-planning-guidance.txt").trim()
    );
    if let Some(hidden) = hidden.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        addendum.push_str(&format!(
            "\n\n<hidden-planning-context>\n{hidden}\n</hidden-planning-context>"
        ));
    }
    lease.planning = Some(Turn {
        submit_tool,
        addendum,
    });
    let command = format!("plan-{}", ag1_storage::digest(input.command.as_bytes()));
    let timestamp = input.timestamp.clone();
    let turn = host
        .run_inner(input, resolver, runner, listener, origin, Some(&lease))
        .await?;
    if lease.cancelled.load(Ordering::Acquire) {
        return Err(DomainError::Cancelled);
    }
    host.wait_for_commit().await?;
    let (stamp, branch) = {
        let state = host.state()?;
        if !state.settled() || !state.persistence_healthy() {
            return Err(DomainError::DurabilityUnknown);
        }
        (state.stamp().clone(), state.branch_snapshot())
    };
    let worker = host.retain_work()?;
    let text = ag1_core::context::hygiene::content(&turn.assistant, "\n");
    let build_command = command.clone();
    let build_timestamp = timestamp.clone();
    let plan = host
        .pool
        .submit(move || {
            let _worker = worker;
            let start = branch
                .iter()
                .rposition(|e| e["message"]["role"] == "user")
                .ok_or(DomainError::Conflict)?;
            let submission = branch[start..]
                .iter()
                .rev()
                .find_map(|e| {
                    let m = &e["message"];
                    (m["role"] == "toolResult"
                        && m["toolName"] == "submit_plan"
                        && m["isError"] != true)
                        .then(|| m["details"].get("plan").cloned())
                        .flatten()
                })
                .map(serde_json::from_value::<Submission>)
                .transpose()
                .map_err(|_| DomainError::InvalidInput("structured planning result".into()))?;
            let structured = submission
                .as_ref()
                .map(|submission| {
                    Plan::from_submission(
                        &build_command,
                        &goal,
                        submission,
                        previous.as_ref(),
                        options.feedback.as_deref(),
                        &build_timestamp,
                    )
                })
                .transpose()?
                .flatten();
            if structured.is_some() || text.trim().is_empty() {
                return Ok(structured);
            }
            let extracted = Plan::from_markdown(
                &build_command,
                &goal,
                &text,
                previous.as_ref(),
                options.feedback.as_deref(),
                &build_timestamp,
            )?;
            if extracted.is_some() {
                return Ok(extracted);
            }
            Plan::fallback(
                &build_command,
                &goal,
                previous.as_ref(),
                options.feedback.as_deref(),
                &text,
                &build_timestamp,
            )
        })
        .await?;
    if lease.cancelled.load(Ordering::Acquire) {
        return Err(DomainError::Cancelled);
    }
    host.wait_for_commit().await?;
    let action_and_plan = {
        let mut state = host.state()?;
        if state.stamp() != &stamp {
            return Err(DomainError::Conflict);
        }
        if let Some(plan) = plan {
            let mut selection = planning::selection(state.branch()).unwrap_or(Selection {
                version: 1,
                planning: None,
                accepted: None,
                executing: None,
                updated_at: timestamp.clone(),
            });
            selection.planning = Some(plan.reference());
            selection.updated_at = timestamp.clone();
            let action = state.update_plan(
                &command,
                &command,
                &timestamp,
                Some(plan.clone()),
                selection,
            )?;
            Some((action, plan))
        } else {
            None
        }
    };
    let plan = if let Some((action, plan)) = action_and_plan {
        host.commit(action, listener).await?;
        Some(plan)
    } else {
        None
    };
    Ok(PlanningResult { turn, plan })
}

impl NativeSession {
    /// Bounded latest-history projection for native clients. The revision and
    /// selections are captured with the plans under the same session lock.
    pub fn plan_snapshot(&self, limit: usize) -> Result<PlanSnapshot, DomainError> {
        if !(1..=128).contains(&limit) {
            return Err(DomainError::InvalidInput("plan history limit".into()));
        }
        let state = self.state()?;
        let mut plans = planning::recent_plans(state.branch(), limit + 1);
        let more = plans.len() > limit;
        plans.truncate(limit);
        Ok(PlanSnapshot {
            session: state.stamp().session().as_str().into(),
            context_epoch: state.stamp().epoch(),
            journal_revision: state.journal_revision(),
            selection: planning::selection(state.branch()),
            plans,
            more,
        })
    }
    pub fn plan(&self, reference: &PlanRef) -> Result<Plan, DomainError> {
        planning::resolve(self.state()?.branch(), reference).ok_or(DomainError::NotFound)
    }
    pub async fn update_plan(
        &self,
        command: &str,
        timestamp: &str,
        plan: Option<Plan>,
        selection: Selection,
        listener: &dyn Listener,
    ) -> Result<(), DomainError> {
        let _activity = self.enter()?;
        if !self.settled()? {
            return Err(DomainError::Conflict);
        }
        let action = self
            .state()?
            .update_plan(command, command, timestamp, plan, selection)?;
        self.commit(action, listener).await
    }
}

#[cfg(test)]
mod file_family_visibility_tests {
    #[test]
    fn planning_hides_every_native_file_mutation_and_keeps_preview() {
        for name in ["write", "edit", "editApply", "append", "notebook_edit"] {
            assert!(!super::Turn::visible(name), "{name}");
        }
        for name in ["read", "editPreview", "diff"] {
            assert!(super::Turn::visible(name), "{name}");
        }
    }
}
