//! Structured planning output. This validates presentation only; submitting a
//! plan never authorizes execution or marks any implementation step complete.
use crate::{DomainError, task::bounded_json};
use serde_json::{Value, json};

fn text(value: &Value) -> Result<String, DomainError> {
    if value.is_null() {
        return Ok(String::new());
    }
    let value = value
        .as_str()
        .ok_or_else(|| DomainError::InvalidInput("plan text".into()))?;
    let mut units = 0;
    Ok(value
        .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 500
        })
        .collect())
}
fn strings(value: &Value) -> Result<Vec<String>, DomainError> {
    let Some(values) = value.as_array() else {
        return Ok(Vec::new());
    };
    values
        .iter()
        .map(text)
        .filter(|r| !r.as_ref().is_ok_and(|s| s.is_empty()))
        .collect()
}
fn error(message: String) -> Value {
    json!({"content":[{"type":"text","text":message}]})
}

pub fn execute(args: &Value) -> Result<Value, DomainError> {
    bounded_json(args)?;
    let summary = text(&args["summary"])?;
    let steps = strings(&args["steps"])?;
    let mut questions = Vec::new();
    if let Some(values) = args["openQuestions"].as_array() {
        for entry in values.iter().filter(|e| e.is_object()) {
            let question = text(&entry["question"])?;
            if question.is_empty() {
                continue;
            }
            let options = strings(&entry["options"])?;
            let mut value = json!({"question":question});
            if !options.is_empty() {
                value["options"] = json!(options);
            }
            questions.push(value);
        }
    }
    if steps.is_empty() && questions.is_empty() {
        return Ok(error("Error: plan has no steps and no open questions. Provide implementation steps, or clarifying questions if foundational choices are open.".into()));
    }
    if steps.len() > 30 {
        return Ok(error(format!(
            "Error: plan has {} steps; the limit is 30. Merge related work into fewer, coarser steps and resubmit.",
            steps.len()
        )));
    }
    if questions.len() > 10 {
        return Ok(error(format!(
            "Error: plan has {} open questions; the limit is 10. Batch related decisions together and resubmit.",
            questions.len()
        )));
    }
    if let Some(question) = questions
        .iter()
        .find(|q| q["options"].as_array().is_some_and(|o| o.len() > 8))
    {
        return Ok(error(format!(
            "Error: question \"{}\" has more than 8 options. Keep only the realistic choices and resubmit.",
            question["question"].as_str().unwrap_or("")
        )));
    }
    let mut plan = json!({"summary":summary,"steps":steps,"openQuestions":questions});
    for field in ["assumptions", "constraints", "risks", "verification"] {
        plan[field] = json!(strings(&args[field])?);
    }
    Ok(
        json!({"content":[{"type":"text","text":format!("Plan submitted: {} steps, {} open questions.",steps.len(),questions.len())}],"details":{"plan":plan}}),
    )
}
