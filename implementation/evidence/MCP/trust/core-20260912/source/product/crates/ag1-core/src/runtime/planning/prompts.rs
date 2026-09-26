use super::*;

fn section(lines: &mut Vec<String>, title: &str, items: &[String], numbered: bool) {
    if items.is_empty() {
        return;
    }
    lines.extend([String::new(), format!("{title}:")]);
    lines.extend(items.iter().enumerate().map(|(i, s)| {
        if numbered {
            format!("{}. {s}", i + 1)
        } else {
            format!("- {s}")
        }
    }));
}
fn name<T: Serialize>(status: T) -> String {
    serde_json::to_value(status)
        .expect("enum serialization")
        .as_str()
        .expect("enum string")
        .into()
}
impl Plan {
    pub fn format(&self) -> String {
        let completed = self.completed_step_indexes.as_deref().unwrap_or(&[]);
        let mut lines = vec![
            format!("Plan {} · {}", self.revision, name(self.status)),
            format!("Goal: {}", self.goal),
            format!(
                "Progress: {}/{} steps complete",
                completed.len(),
                self.steps.len()
            ),
        ];
        for (label, value) in [
            ("Feedback", &self.feedback),
            ("Rejected", &self.rejected_reason),
            ("Execution Step", &self.execution_step),
        ] {
            if let Some(value) = value {
                lines.push(format!("{label}: {value}"));
            }
        }
        let reviews = self.reviews();
        let review = reviews.last();
        if let Some(r) = review {
            lines.push(format!(
                "Latest Review: step {} · {}{}",
                r.step_index + 1,
                name(r.status),
                r.summary
                    .as_ref()
                    .map(|s| format!(" · {s}"))
                    .unwrap_or_default()
            ));
        }
        if let Some(summary) = &self.summary {
            lines.extend([String::new(), "Summary:".into(), summary.clone()]);
        }
        let steps = self
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "[{}] {s}",
                    if completed.contains(&i) {
                        "done"
                    } else if self.next_step() == Some(i) {
                        "next"
                    } else {
                        "pending"
                    }
                )
            })
            .collect::<Vec<_>>();
        section(&mut lines, "Steps", &steps, true);
        if self.status == Status::Clarifying && !self.open_questions.is_empty() {
            section(
                &mut lines,
                "Clarifying Questions",
                &self.open_questions,
                false,
            );
            let answers = self
                .open_questions
                .iter()
                .enumerate()
                .flat_map(|(i, q)| {
                    self.clarification_answers
                        .as_ref()
                        .and_then(|a| a.get(i))
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(|s| vec![format!("Q{}: {q}", i + 1), format!("A{}: {s}", i + 1)])
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>();
            section(&mut lines, "Clarification Answers", &answers, false);
        } else {
            section(&mut lines, "Open Questions", &self.open_questions, false);
        }
        for (title, items) in [
            ("Assumptions", &self.assumptions),
            ("Constraints", &self.constraints),
            ("Risks", &self.risks),
            ("Verification", &self.verification),
        ] {
            section(&mut lines, title, items, false);
        }
        if let Some(r) = review {
            let mut lines_review = vec![format!("Step {} · {}", r.step_index + 1, name(r.status))];
            if let Some(summary) = &r.summary {
                lines_review.push(summary.clone());
            }
            for (title, items, separator) in [
                ("Files", &r.files_touched, ", "),
                ("Verification", &r.verification, "; "),
                ("Remaining", &r.remaining, "; "),
                ("Blockers", &r.blockers, "; "),
            ] {
                if !items.is_empty() {
                    lines_review.push(format!("{title}: {}", items.join(separator)));
                }
            }
            section(&mut lines, "Recent Execution Review", &lines_review, false);
        }
        lines.join("\n")
    }
    pub fn revision_prompt(&self, feedback: &str) -> String {
        format!(
            "Revise the saved execution plan for this goal: {}\n\nRevision requirements:\n- stay in planning mode and do not execute anything\n- preserve the good parts of the current plan\n- address the feedback directly\n- return a full revised plan, not just deltas\n- structure the answer with: Summary, Assumptions, Constraints, Steps, Risks, Verification\n\nCurrent saved plan:\n{}\n\nUser feedback:\n{}",
            self.goal,
            self.raw_plan,
            feedback.trim()
        )
    }
    pub fn clarification_prompt(&self, answer: &str) -> String {
        let interview = self
            .open_questions
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let answer = self
                    .clarification_answers
                    .as_ref()
                    .and_then(|a| a.get(i))
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("(unanswered)");
                format!("Q{}: {q}\nA{}: {answer}", i + 1, i + 1)
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        format!(
            "Continue planning for this goal: {}\n\nPlanning requirements:\n- stay in planning mode and do not execute anything\n- use the saved clarifying questions and the user's answers\n- return a full revised plan, not just notes\n- structure the answer with: Summary, Assumptions, Constraints, Steps, Risks, Verification\n\nCurrent planning state:\n{}{}\n\nUser clarification:\n{}",
            self.goal,
            self.format(),
            if interview.is_empty() {
                String::new()
            } else {
                format!("\n\nSaved interview answers:\n{interview}")
            },
            answer.trim()
        )
    }
    pub fn execution_prompt(&self, index: Option<usize>) -> Result<String, DomainError> {
        self.validate()?;
        let mut lines = if let Some(index) = index {
            self.steps.get(index).ok_or(DomainError::NotFound)?;
            vec![format!("Execute only step {} of the accepted plan for this goal: {}",index+1,self.goal),String::new(),"Execution requirements:".into(),
                "- focus only on the selected step".into(),"- do not try to complete the whole plan in one pass".into(),"- make real progress on this step".into(),"- verify the result of this step before stopping".into(),"- briefly note what remains for the later steps".into(),"- treat any already-completed steps as done unless the work proves they must be reopened".into()]
        } else {
            vec![
                format!("Execute the accepted plan for this goal: {}", self.goal),
                String::new(),
                "Execution requirements:".into(),
                "- do not restate the full plan unless needed".into(),
                "- continue from the next incomplete step and make real progress".into(),
                "- use tools as needed".into(),
                "- verify work as you go".into(),
                "- stop after the next meaningful slice if the task is not complete".into(),
            ]
        };
        lines.extend(["- finish with exact markdown headings: Execution Status, Execution Summary, Files Touched, Verification, Remaining, Blockers".into(),"- under Execution Status, use exactly one of: reviewed, blocked, completed".into(),"- under Files Touched, Verification, Remaining, and Blockers, use bullet lists".into()]);
        if let Some(index) = index {
            lines.extend([
                String::new(),
                format!("Selected step: {}", self.steps[index]),
            ]);
        } else if let Some(index) = self.next_step() {
            lines.extend([
                String::new(),
                format!("Next incomplete step: {}. {}", index + 1, self.steps[index]),
            ]);
        }
        lines.extend([String::new(), "Accepted plan:".into(), self.format()]);
        Ok(lines.join("\n"))
    }
}
