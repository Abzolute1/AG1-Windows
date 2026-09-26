use super::*;
use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Submission {
    pub summary: Option<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<Question>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub verification: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
}
fn items(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .collect()
}
fn optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
fn goal(value: &str) -> String {
    static ALIASES: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b(planmode|planningmode)\b").unwrap());
    static SUBJECT: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
        r"\b(?:enter|open|start|use|switch to|go into)\s+(?:planning|plan)\s+mode\s+(?:for|to plan)\s+(.+)$",
        r"\b(?:let(?:'s| us)?\s+)?plan(?:\s+out)?\s+(.+)$",
        r"\b(?:help me\s+)?plan(?:\s+out)?\s+(.+)$",
        r"\b(?:build|create|make|implement|design|scaffold|start)\s+(.+)$",
    ].map(|r| Regex::new(r).unwrap()).into()
    });
    let trimmed = value.trim();
    let lower = trimmed.to_lowercase();
    let expanded = ALIASES.replace_all(&lower, |c: &regex::Captures<'_>| {
        if &c[1] == "planmode" {
            "plan mode"
        } else {
            "planning mode"
        }
    });
    let normalized = expanded.split_whitespace().collect::<Vec<_>>().join(" ");
    let subject = SUBJECT
        .iter()
        .find_map(|re| re.captures(&normalized))
        .map(|c| c[1].trim().to_owned());
    let Some(subject) = subject.filter(|s| s != &normalized) else {
        return trimmed.into();
    };
    // Preserve the source's case and original spelling, without slicing UTF-8
    // at an index measured in a different string.
    if let Some(index) = expanded.rfind(&subject) {
        let before = expanded[..index].chars().count();
        let original: String = trimmed.chars().skip(before).collect();
        return original
            .trim_matches(|c: char| c.is_whitespace() || ",.:;!?-".contains(c))
            .into();
    }
    trimmed.into()
}
impl Plan {
    /// Build from a schema-checked structured tool submission. The host supplies
    /// a stable ID and timestamp so retry never creates a second plan thread.
    pub fn from_submission(
        plan_id: &str,
        instruction: &str,
        submission: &Submission,
        previous: Option<&Plan>,
        feedback: Option<&str>,
        timestamp: &str,
    ) -> Result<Option<Self>, DomainError> {
        bounded_json(&serde_json::to_value(submission).map_err(|_| DomainError::Conflict)?)?;
        if let Some(previous) = previous {
            previous.validate()?;
        }
        let steps = items(&submission.steps);
        let questions: Vec<_> = submission
            .open_questions
            .iter()
            .filter_map(|q| {
                let question = q.question.split_whitespace().collect::<Vec<_>>().join(" ");
                (!question.is_empty()).then(|| {
                    (
                        question,
                        items(&q.options).into_iter().take(8).collect::<Vec<_>>(),
                    )
                })
            })
            .collect();
        if steps.is_empty() && questions.is_empty() {
            return Ok(None);
        }
        let mut plan = Self {
            version: 1,
            plan_id: previous
                .map(|p| p.plan_id.as_str())
                .unwrap_or(plan_id)
                .into(),
            revision: previous
                .map_or(0, |p| p.revision)
                .checked_add(1)
                .ok_or(DomainError::Conflict)?,
            status: if steps.is_empty() {
                Status::Clarifying
            } else {
                Status::Draft
            },
            goal: goal(instruction),
            summary: optional(submission.summary.as_deref()),
            assumptions: items(&submission.assumptions),
            constraints: items(&submission.constraints),
            steps,
            risks: items(&submission.risks),
            verification: items(&submission.verification),
            open_questions: questions.iter().map(|(q, _)| q.clone()).collect(),
            clarification_option_sets: Some(questions.into_iter().map(|(_, o)| o).collect()),
            clarification_answers: previous.and_then(|p| p.clarification_answers.clone()),
            raw_plan: String::new(),
            feedback: optional(feedback),
            rejected_reason: None,
            completed_step_indexes: None,
            execution_step_index: None,
            execution_step: None,
            step_execution_records: None,
            created_at: previous
                .map(|p| p.created_at.as_str())
                .unwrap_or(timestamp)
                .into(),
            updated_at: timestamp.into(),
        };
        let mut sections = Vec::new();
        if let Some(summary) = &plan.summary {
            sections.push(format!("Summary:\n{summary}"));
        }
        for (title, values, numbered) in [
            ("Steps", &plan.steps, true),
            ("Clarifying Questions", &plan.open_questions, true),
            ("Assumptions", &plan.assumptions, false),
            ("Constraints", &plan.constraints, false),
            ("Risks", &plan.risks, false),
            ("Verification", &plan.verification, false),
        ] {
            if !values.is_empty() {
                let lines = values
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        if numbered {
                            format!("{}. {s}", i + 1)
                        } else {
                            format!("- {s}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                sections.push(format!("{title}:\n{lines}"));
            }
        }
        plan.raw_plan = sections.join("\n\n");
        plan.validate()?;
        Ok(Some(plan))
    }
}
