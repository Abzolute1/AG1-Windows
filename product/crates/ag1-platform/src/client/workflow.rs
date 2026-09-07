//! Pinned interactive workflow behavior: validate a file and steer text. This
//! does not adopt the separate, unintegrated DAG runner.
use super::*;
#[derive(Deserialize)]
struct Workflow {
    name: String,
    steps: Vec<Step>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    prompt: String,
    #[serde(default)]
    depends_on: Vec<String>,
}
pub fn text(bytes: &[u8]) -> Result<String> {
    if bytes.len() > 32768 {
        return Err(DomainError::LimitExceeded("workflow file".into()));
    }
    let workflow: Workflow = serde_json::from_slice(bytes).map_err(json_error)?;
    if workflow.steps.len() > 128 {
        return Err(DomainError::LimitExceeded("workflow steps".into()));
    }
    let mut ids = std::collections::BTreeSet::new();
    for step in &workflow.steps {
        if !ids.insert(step.id.clone()) {
            return Err(DomainError::InvalidInput(format!(
                "Duplicate step ID: {}",
                step.id
            )));
        }
        if step.prompt.is_empty() {
            return Err(DomainError::InvalidInput(format!(
                "Step {} is missing a prompt",
                step.id
            )));
        }
    }
    for step in &workflow.steps {
        for dependency in &step.depends_on {
            if !ids.contains(dependency) {
                return Err(DomainError::InvalidInput(format!(
                    "Step {} depends on unknown step {}",
                    step.id, dependency
                )));
            }
        }
    }
    let mut complete = std::collections::BTreeSet::new();
    while complete.len() < workflow.steps.len() {
        let before = complete.len();
        for step in &workflow.steps {
            if step.depends_on.iter().all(|id| complete.contains(id)) {
                complete.insert(step.id.clone());
            }
        }
        if complete.len() == before {
            return Err(DomainError::InvalidInput(
                "Circular workflow dependency".into(),
            ));
        }
    }
    let mut lines = vec![format!(
        "Execute the following workflow \"{}\":",
        workflow.name
    )];
    for step in &workflow.steps {
        let dependencies = if step.depends_on.is_empty() {
            String::new()
        } else {
            format!(" (depends on: {})", step.depends_on.join(", "))
        };
        lines.push(format!(
            "Step \"{}\" ({}){}: {}",
            step.id, step.kind, dependencies, step.prompt
        ));
    }
    lines.extend([String::new(),"Execute each step using the task tool. Wait for dependent steps to complete before starting their dependents.".into()]);
    Ok(lines.join("\n"))
}
impl Host {
    pub(super) fn workflow(&self, args: &Value) -> Result<w::Command> {
        let path = args["path"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| DomainError::InvalidInput("/workflow <path-to-workflow.json>".into()))?;
        let file = self.workspace.snapshot(path, 32768)?;
        Ok(w::Command::Session {
            action: "steer".into(),
            args: json!({"input":{"kind":"inline","text":text(&file.bytes)?}}),
        })
    }
}
