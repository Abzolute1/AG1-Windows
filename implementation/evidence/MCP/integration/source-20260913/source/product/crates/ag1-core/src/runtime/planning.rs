//! Reviewed plans are durable workflow state, separate from verification
//! evidence and tool authorization. A selected plan does not grant execution.
use crate::{DomainError, task::bounded_json};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod submission;
pub use submission::{Question, Submission};
mod steps;
pub use steps::{Review, ReviewStatus};
mod markdown;
mod prompts;
pub use markdown::{allow_inspection, should_interview};

pub const ARTIFACT: &str = "interactive_plan_artifact";
pub const SELECTION: &str = "interactive_plan_selection";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Draft,
    Clarifying,
    Accepted,
    Rejected,
    Superseded,
    Discarded,
    Executing,
    Completed,
}
impl Status {
    pub fn allows(self, next: Self) -> bool {
        use Status::*;
        match self {
            Draft => matches!(next, Draft | Accepted | Rejected | Discarded | Superseded),
            Clarifying => matches!(next, Clarifying | Draft | Rejected | Discarded | Superseded),
            Accepted => matches!(
                next,
                Accepted | Executing | Completed | Rejected | Discarded | Superseded
            ),
            Rejected => matches!(next, Rejected | Discarded),
            Superseded => matches!(next, Superseded | Discarded),
            Discarded => next == Discarded,
            Executing => matches!(
                next,
                Executing | Accepted | Completed | Rejected | Discarded
            ),
            Completed => matches!(next, Completed | Accepted | Discarded),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRef {
    pub plan_id: String,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planning: Option<PlanRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<PlanRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executing: Option<PlanRef>,
    pub updated_at: String,
}
impl Selection {
    pub fn validate(&self) -> Result<(), DomainError> {
        bounded_json(&serde_json::to_value(self).map_err(|_| DomainError::Conflict)?)?;
        if self.version != 1
            || [&self.planning, &self.accepted, &self.executing]
                .into_iter()
                .flatten()
                .any(|r| r.plan_id.is_empty() || r.plan_id.len() > 1024 || r.revision == 0)
        {
            return Err(DomainError::InvalidInput("plan selection".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub version: u32,
    pub plan_id: String,
    pub revision: u64,
    pub status: Status,
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub verification: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clarification_option_sets: Option<Vec<Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clarification_answers: Option<Vec<String>>,
    pub raw_plan: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_step_indexes: Option<Vec<usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_step_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_execution_records: Option<Vec<Value>>,
    pub created_at: String,
    pub updated_at: String,
}
impl Plan {
    pub fn reference(&self) -> PlanRef {
        PlanRef {
            plan_id: self.plan_id.clone(),
            revision: self.revision,
        }
    }
    pub fn validate(&self) -> Result<(), DomainError> {
        bounded_json(&serde_json::to_value(self).map_err(|_| DomainError::Conflict)?)?;
        let invalid = self.version != 1
            || self.plan_id.is_empty()
            || self.plan_id.len() > 1024
            || self.revision == 0
            || self.status == Status::Clarifying
                && (!self.steps.is_empty() || self.open_questions.is_empty())
            || matches!(
                self.status,
                Status::Draft | Status::Accepted | Status::Executing
            ) && self.steps.is_empty()
            || self.clarification_answers.as_ref().is_some_and(|answers| {
                !self.open_questions.is_empty() && answers.len() > self.open_questions.len()
            })
            || self
                .completed_step_indexes
                .as_ref()
                .is_some_and(|ids| ids.iter().any(|&id| id >= self.steps.len()));
        let invalid_execution = if self.status == Status::Executing {
            self.execution_step_index
                .and_then(|i| self.steps.get(i))
                .is_none_or(|step| Some(step) != self.execution_step.as_ref())
        } else {
            self.execution_step_index.is_some() || self.execution_step.is_some()
        };
        if invalid || invalid_execution {
            return Err(DomainError::InvalidInput("plan state".into()));
        }
        Ok(())
    }
    pub fn transition(&self, next: Status, timestamp: &str) -> Result<Self, DomainError> {
        self.validate()?;
        if !self.status.allows(next) {
            return Err(DomainError::Conflict);
        }
        let mut plan = self.clone();
        plan.status = next;
        plan.updated_at = timestamp.into();
        if next != Status::Executing {
            plan.execution_step_index = None;
            plan.execution_step = None;
        }
        plan.validate()?;
        Ok(plan)
    }
    pub fn answer(&self, index: usize, answer: &str, timestamp: &str) -> Result<Self, DomainError> {
        self.validate()?;
        if self.status != Status::Clarifying || index >= self.open_questions.len() {
            return Err(DomainError::Conflict);
        }
        let mut plan = self.clone();
        let answers = plan.clarification_answers.get_or_insert_with(Vec::new);
        answers.resize(self.open_questions.len(), String::new());
        answers[index] = answer.trim().into();
        plan.updated_at = timestamp.into();
        plan.validate()?;
        Ok(plan)
    }
    pub fn begin_step(&self, index: usize, timestamp: &str) -> Result<Self, DomainError> {
        self.validate()?;
        let step = self.steps.get(index).ok_or(DomainError::NotFound)?;
        if !self.status.allows(Status::Executing) {
            return Err(DomainError::Conflict);
        }
        let mut plan = self.clone();
        plan.status = Status::Executing;
        plan.execution_step_index = Some(index);
        plan.execution_step = Some(step.clone());
        plan.updated_at = timestamp.into();
        plan.validate()?;
        Ok(plan)
    }
}

/// Latest valid state for each exact plan/revision pair. Revision numbers alone
/// cannot choose between independent plan threads.
pub fn plans(branch: &[Value]) -> Vec<Plan> {
    recent_plans(branch, usize::MAX)
}
pub fn recent_plans(branch: &[Value], limit: usize) -> Vec<Plan> {
    let mut seen = std::collections::BTreeSet::new();
    branch
        .iter()
        .rev()
        .filter(|e| e["type"] == "custom" && e["customType"] == ARTIFACT)
        .filter_map(|e| serde_json::from_value::<Plan>(e["data"].clone()).ok())
        .filter(|p| p.validate().is_ok() && seen.insert((p.plan_id.clone(), p.revision)))
        .take(limit)
        .collect()
}
pub fn selection(branch: &[Value]) -> Option<Selection> {
    branch
        .iter()
        .rev()
        .filter(|e| e["type"] == "custom" && e["customType"] == SELECTION)
        .filter_map(|e| serde_json::from_value::<Selection>(e["data"].clone()).ok())
        .find(|s| s.validate().is_ok())
}
pub fn resolve(branch: &[Value], reference: &PlanRef) -> Option<Plan> {
    branch
        .iter()
        .rev()
        .filter(|e| {
            e["type"] == "custom"
                && e["customType"] == ARTIFACT
                && e["data"]["planId"] == reference.plan_id
                && e["data"]["revision"].as_u64() == Some(reference.revision)
        })
        .filter_map(|e| serde_json::from_value::<Plan>(e["data"].clone()).ok())
        .find(|p| p.validate().is_ok())
}
