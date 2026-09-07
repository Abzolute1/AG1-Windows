use super::*;
use crate::runtime::planning::{self, Plan, Selection, Status};

impl SessionRuntime {
    /// Publish a plan update and its role selections in one acknowledged branch
    /// transaction. This is workflow state, never verification or tool consent.
    pub fn update_plan(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        plan: Option<Plan>,
        selection: Selection,
    ) -> Result<AppendAction, DomainError> {
        if !self.settled() || !self.persistence_healthy() {
            return Err(DomainError::Conflict);
        }
        selection.validate()?;
        SessionId::new(command)?;
        SessionId::new(id)?;
        if chrono::DateTime::parse_from_rfc3339(timestamp).is_err() {
            return Err(invalid("plan timestamp"));
        }
        let mut values = Vec::new();
        if let Some(plan) = plan {
            plan.validate()?;
            if let Some(current) = planning::resolve(&self.branch, &plan.reference()) {
                if !current.status.allows(plan.status) {
                    return Err(DomainError::Conflict);
                }
            } else {
                let plans = planning::plans(&self.branch);
                let previous = plans
                    .iter()
                    .filter(|p| p.plan_id == plan.plan_id)
                    .map(|p| p.revision)
                    .max()
                    .unwrap_or(0);
                if previous.checked_add(1) != Some(plan.revision)
                    || !matches!(plan.status, Status::Draft | Status::Clarifying)
                {
                    return Err(DomainError::Conflict);
                }
                if let Some(previous) = plans
                    .iter()
                    .find(|p| p.plan_id == plan.plan_id && p.revision == previous)
                    && matches!(previous.status, Status::Draft | Status::Clarifying)
                {
                    values.push(json!({"type":"custom","customType":planning::ARTIFACT,"data":previous.transition(Status::Superseded,timestamp)?}));
                }
            }
            values.push(json!({"type":"custom","customType":planning::ARTIFACT,"data":plan}));
        }
        let mut projected = self.branch.as_ref().clone();
        projected.extend(values.iter().cloned());
        for (reference, statuses) in [
            (
                &selection.planning,
                &[Status::Draft, Status::Clarifying][..],
            ),
            (
                &selection.accepted,
                &[Status::Accepted, Status::Executing, Status::Completed][..],
            ),
            (&selection.executing, &[Status::Executing][..]),
        ] {
            if let Some(reference) = reference {
                let selected =
                    planning::resolve(&projected, reference).ok_or(DomainError::NotFound)?;
                if !statuses.contains(&selected.status) {
                    return Err(DomainError::Conflict);
                }
            }
        }
        values.push(json!({"type":"custom","customType":planning::SELECTION,"data":selection}));
        if self.branch.len() + values.len() > 100_000 {
            return Err(DomainError::LimitExceeded("session branch".into()));
        }
        let mut branch = self.branch.as_ref().clone();
        let mut records = Vec::new();
        for (index, mut value) in values.into_iter().enumerate() {
            let entry_id = format!("{id}-{index}");
            SessionId::new(&entry_id)?;
            if branch.iter().any(|e| e["id"] == entry_id) {
                return Err(DomainError::Conflict);
            }
            value["id"] = json!(entry_id);
            value["parentId"] = branch
                .last()
                .map(|e| e["id"].clone())
                .unwrap_or(Value::Null);
            value["timestamp"] = json!(timestamp);
            bounded_json(&value)?;
            records.push(serde_json::to_vec(&value).map_err(|_| DomainError::Conflict)?);
            branch.push(value);
        }
        let messages = ContextSnapshot::capture(context_messages(&branch))?;
        let token = self.boundary.begin_append()?;
        self.pending_commit = true;
        Ok(AppendAction {
            token,
            write: PreparedStateCommit {
                session: self.stamp().session().clone(),
                command: command.into(),
                expected: self.journal_revision,
                records,
            },
            next_branch: Arc::new(branch),
            next_messages: messages,
            next_configuration: self.configuration.clone(),
            tools: ToolTransition::Keep,
            compaction: None,
        })
    }
}
