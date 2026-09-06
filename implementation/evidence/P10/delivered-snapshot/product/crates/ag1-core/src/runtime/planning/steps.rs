//! Plan reviews describe workflow progress. Their verification strings remain
//! assistant claims; they cannot create host verification evidence.
use super::*;
use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewStatus {
    Reviewed,
    Blocked,
    Completed,
    Reopened,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub step_index: usize,
    pub step: String,
    pub status: ReviewStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub files_touched: Vec<String>,
    #[serde(default)]
    pub verification: Vec<String>,
    #[serde(default)]
    pub remaining: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    pub recorded_at: String,
}
fn clean(value: &str) -> String {
    static EMPHASIS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\*{1,2}([^*]+)\*{1,2}").unwrap());
    static CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").unwrap());
    CODE.replace_all(&EMPHASIS.replace_all(value, "$1"), "$1")
        .trim()
        .into()
}
impl Review {
    pub fn parse(
        text: &str,
        step_index: usize,
        step: &str,
        timestamp: &str,
    ) -> Result<Option<Self>, DomainError> {
        bounded_json(&Value::String(text.into()))?;
        static HEADING: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)\n(?:#{1,6}\s*)?(execution status|execution summary|step summary|review summary|files touched|touched files|changed files|files|verification|verified|validation|remaining|remaining work|next|follow-up|blockers|blocked|issues)\s*:?[ \t]*\n").unwrap()
        });
        let text = text.trim();
        if text.is_empty() || step.is_empty() {
            return Ok(None);
        }
        let section = |names: &[&str]| -> &str {
            let alternatives = names
                .iter()
                .map(|s| regex::escape(s))
                .collect::<Vec<_>>()
                .join("|");
            let start = Regex::new(&format!(
                r"(?i)(?:^|\n)(?:#{{1,6}}\s*)?(?:{alternatives})\s*:?[ \t]*\n"
            ))
            .unwrap();
            let Some(found) = start.find(text) else {
                return "";
            };
            let body = &text[found.end()..];
            body[..HEADING.find(body).map_or(body.len(), |next| next.start())].trim()
        };
        let status = section(&["execution status"]);
        let summary = clean(section(&[
            "execution summary",
            "step summary",
            "review summary",
        ]));
        if status.is_empty() || summary.is_empty() {
            return Ok(None);
        }
        let bullets = |names: &[&str]| -> Vec<String> {
            static BULLET: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"(?m)^\s*[-*]\s+(.+)$").unwrap());
            BULLET
                .captures_iter(section(names))
                .map(|c| clean(&c[1]))
                .filter(|s| {
                    !s.is_empty()
                        && ![
                            "none",
                            "n/a",
                            "no blockers",
                            "no remaining work",
                            "no files",
                            "no files touched",
                        ]
                        .contains(&s.to_lowercase().as_str())
                })
                .collect()
        };
        let blockers = bullets(&["blockers", "blocked", "issues"]);
        static STATUS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
            [
                r"\bblock(ed)?\b",
                r"\bcomplete(d)?\b",
                r"\breview(ed)?\b|\bpartial\b|\bin progress\b",
            ]
            .map(|s| Regex::new(s).unwrap())
            .into()
        });
        let normalized = status
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let status = if STATUS[0].is_match(&normalized) {
            ReviewStatus::Blocked
        } else if STATUS[1].is_match(&normalized) {
            ReviewStatus::Completed
        } else if STATUS[2].is_match(&normalized) {
            ReviewStatus::Reviewed
        } else if !blockers.is_empty() {
            ReviewStatus::Blocked
        } else {
            return Ok(None);
        };
        Ok(Some(Self {
            step_index,
            step: step.into(),
            status,
            summary: Some(summary),
            files_touched: bullets(&["files touched", "touched files", "changed files", "files"]),
            verification: bullets(&["verification", "verified", "validation"]),
            remaining: bullets(&["remaining", "remaining work", "next", "follow-up"]),
            blockers,
            recorded_at: timestamp.into(),
        }))
    }
}
impl Plan {
    pub fn reviews(&self) -> Vec<Review> {
        self.step_execution_records
            .iter()
            .flatten()
            .filter_map(|v| serde_json::from_value::<Review>(v.clone()).ok())
            .filter(|r| {
                self.steps
                    .get(r.step_index)
                    .is_some_and(|step| step == &r.step)
            })
            .collect()
    }
    pub fn next_step(&self) -> Option<usize> {
        (0..self.steps.len()).find(|i| {
            !self
                .completed_step_indexes
                .as_ref()
                .is_some_and(|ids| ids.contains(i))
        })
    }
    pub fn mark_step(
        &self,
        index: usize,
        completed: bool,
        timestamp: &str,
    ) -> Result<Self, DomainError> {
        self.validate()?;
        if !matches!(self.status, Status::Accepted | Status::Executing)
            && (completed || self.status != Status::Completed)
        {
            return Err(DomainError::Conflict);
        }
        let step = self.steps.get(index).ok_or(DomainError::NotFound)?;
        let mut result = self.clone();
        let ids = result.completed_step_indexes.get_or_insert_default();
        ids.retain(|&i| i != index);
        if completed {
            ids.push(index);
        }
        ids.sort_unstable();
        ids.dedup();
        result.status = if ids.len() == self.steps.len() {
            Status::Completed
        } else {
            Status::Accepted
        };
        result.execution_step_index = None;
        result.execution_step = None;
        result.updated_at = timestamp.into();
        let mut reviews = self.reviews();
        let status = if completed {
            ReviewStatus::Completed
        } else {
            ReviewStatus::Reopened
        };
        if let Some(review) = reviews.iter_mut().rev().find(|r| r.step_index == index) {
            review.status = status;
            review.recorded_at = timestamp.into();
        } else {
            reviews.push(Review {
                step_index: index,
                step: step.clone(),
                status,
                summary: Some(
                    if completed {
                        "Marked complete from plan review."
                    } else {
                        "Reopened for more work."
                    }
                    .into(),
                ),
                files_touched: vec![],
                verification: vec![],
                remaining: vec![],
                blockers: vec![],
                recorded_at: timestamp.into(),
            });
        }
        result.step_execution_records = Some(
            reviews
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|_| DomainError::Conflict)?,
        );
        result.validate()?;
        Ok(result)
    }
    pub fn record_review(
        &self,
        index: usize,
        text: &str,
        timestamp: &str,
    ) -> Result<Option<Self>, DomainError> {
        self.validate()?;
        if self.status != Status::Executing || self.execution_step_index != Some(index) {
            return Err(DomainError::Conflict);
        }
        let step = self.steps.get(index).ok_or(DomainError::NotFound)?;
        let Some(review) = Review::parse(text, index, step, timestamp)? else {
            return Ok(None);
        };
        let mut result = self.clone();
        if review.status == ReviewStatus::Completed {
            let ids = result.completed_step_indexes.get_or_insert_default();
            ids.push(index);
            ids.sort_unstable();
            ids.dedup();
            // A completed step leaves execution. Keeping executing here without
            // execution fields would invalidate the record on the next read.
            result.status = if ids.len() == self.steps.len() {
                Status::Completed
            } else {
                Status::Accepted
            };
            result.execution_step_index = None;
            result.execution_step = None;
        }
        result
            .step_execution_records
            .get_or_insert_default()
            .push(serde_json::to_value(review).map_err(|_| DomainError::Conflict)?);
        result.updated_at = timestamp.into();
        result.validate()?;
        Ok(Some(result))
    }
}
