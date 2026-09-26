use super::*;
use ag1_core::runtime::planning::{self, Plan, PlanRef, Selection, Status};
use serde_json::Value;

#[derive(Clone)]
pub(super) enum Request {
    Draft {
        previous: Option<PlanRef>,
        feedback: Option<String>,
    },
    Execute {
        plan: PlanRef,
        step_index: Option<usize>,
    },
}
pub(super) fn queued(
    command: &w::Command,
) -> Result<Option<(Request, w::PromptInput)>, w::ProtocolError> {
    let w::Command::Plan { action, args } = command else {
        return Ok(None);
    };
    let invalid = || w::ProtocolError::Malformed {
        detail: "planning turn arguments".into(),
    };
    Ok(match action.as_str() {
        "draft" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Draft {
                input: w::PromptInput,
                previous: Option<PlanRef>,
                feedback: Option<String>,
            }
            let draft: Draft = serde_json::from_value(args.clone()).map_err(|_| invalid())?;
            Some((
                Request::Draft {
                    previous: draft.previous,
                    feedback: draft.feedback,
                },
                draft.input,
            ))
        }
        "execute" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Execute {
                plan_id: String,
                revision: u64,
                step_index: Option<usize>,
            }
            let execution: Execute = serde_json::from_value(args.clone()).map_err(|_| invalid())?;
            Some((
                Request::Execute {
                    plan: PlanRef {
                        plan_id: execution.plan_id,
                        revision: execution.revision,
                    },
                    step_index: execution.step_index,
                },
                w::PromptInput::Inline {
                    text: String::new(),
                },
            ))
        }
        _ => None,
    })
}

pub(super) struct Exclusive<'a>(&'a SessionCommands);
impl Drop for Exclusive<'_> {
    fn drop(&mut self) {
        if let Ok(mut book) = self.0.book() {
            book.exclusive_control = false;
        }
    }
}
impl SessionCommands {
    pub(super) fn exclusive_plan_control(&self) -> Result<Exclusive<'_>, w::ProtocolError> {
        let mut book = self.book().map_err(domain)?;
        if book.exclusive_control
            || book.active.is_some()
            || !self.runtime().settled().map_err(domain)?
        {
            return Err(unavailable("native_plan_busy"));
        }
        book.exclusive_control = true;
        Ok(Exclusive(self))
    }
    pub(super) fn plan_change(
        &self,
        action: &str,
        args: &Value,
        timestamp: &str,
    ) -> Result<(Option<Plan>, Selection), w::ProtocolError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Args {
            plan_id: String,
            revision: u64,
            #[serde(default)]
            reason: Option<String>,
            #[serde(default)]
            question_index: Option<usize>,
            #[serde(default)]
            answer: Option<String>,
            #[serde(default)]
            step_index: Option<usize>,
        }
        let args: Args =
            serde_json::from_value(args.clone()).map_err(|_| w::ProtocolError::Malformed {
                detail: "plan arguments".into(),
            })?;
        let reference = PlanRef {
            plan_id: args.plan_id,
            revision: args.revision,
        };
        let state = self.runtime().state().map_err(domain)?;
        let plan =
            planning::resolve(state.branch(), &reference).ok_or(w::ProtocolError::NotFound)?;
        let mut selection = planning::selection(state.branch()).unwrap_or(Selection {
            version: 1,
            planning: None,
            accepted: None,
            executing: None,
            updated_at: timestamp.into(),
        });
        let updated = match action {
            "accept"
                if plan.status == Status::Draft
                    && args.reason.is_none()
                    && args.question_index.is_none()
                    && args.step_index.is_none()
                    && args.answer.is_none() =>
            {
                let plan = plan
                    .transition(Status::Accepted, timestamp)
                    .map_err(domain)?;
                selection.planning = None;
                selection.executing = None;
                selection.accepted = Some(reference);
                plan
            }
            "reject" | "clear"
                if args.question_index.is_none()
                    && args.step_index.is_none()
                    && args.answer.is_none()
                    && (action == "reject" || args.reason.is_none()) =>
            {
                let mut plan = plan
                    .transition(
                        if action == "reject" {
                            Status::Rejected
                        } else {
                            Status::Discarded
                        },
                        timestamp,
                    )
                    .map_err(domain)?;
                if let Some(reason) = args.reason {
                    plan.rejected_reason = Some(reason.trim().into());
                }
                for selected in [
                    &mut selection.planning,
                    &mut selection.accepted,
                    &mut selection.executing,
                ] {
                    if selected.as_ref() == Some(&reference) {
                        *selected = None;
                    }
                }
                plan
            }
            "complete_step" | "reopen_step"
                if args.reason.is_none()
                    && args.answer.is_none()
                    && args.question_index.is_none() =>
            {
                let plan = plan
                    .mark_step(
                        args.step_index.ok_or(w::ProtocolError::NotFound)?,
                        action == "complete_step",
                        timestamp,
                    )
                    .map_err(domain)?;
                selection.accepted = Some(reference);
                selection.executing = None;
                plan
            }
            "answer" if args.reason.is_none() && args.step_index.is_none() => plan
                .answer(
                    args.question_index.ok_or(w::ProtocolError::NotFound)?,
                    args.answer.as_deref().ok_or(w::ProtocolError::NotFound)?,
                    timestamp,
                )
                .map_err(domain)?,
            _ => {
                return Err(w::ProtocolError::Malformed {
                    detail: "plan action or state".into(),
                });
            }
        };
        selection.updated_at = timestamp.into();
        updated.validate().map_err(domain)?;
        Ok((Some(updated), selection))
    }
}
