//! Shared exploration board and bounded report projection. Raw legacy board
//! bytes remain in the import archive; corrupt boards require inspection.
use crate::{DomainError, task::bounded_json};
use serde_json::{Value, json};

pub fn questions(args: &Value) -> Result<Vec<String>, DomainError> {
    let raw: Vec<String> =
        serde_json::from_value(args["questions"].clone()).map_err(|_| DomainError::Conflict)?;
    if raw.len() > 4 {
        return Err(DomainError::InvalidInput(
            "Exploration question count".into(),
        ));
    }
    let questions: Vec<String> = raw
        .into_iter()
        .map(|q| q.trim().to_owned())
        .filter(|q| !q.is_empty())
        .collect();
    if questions.is_empty() {
        return Err(DomainError::InvalidInput(
            "Exploration questions are empty".into(),
        ));
    }
    Ok(questions)
}

pub fn board(value: &Value) -> Result<Value, DomainError> {
    bounded_json(value)?;
    if value["version"] != 1
        || value["findings"].as_array().is_none_or(|rows| {
            rows.len() > 4096
                || rows
                    .iter()
                    .any(|r| !r["question"].is_string() || !r["summary"].is_string())
        })
    {
        return Err(DomainError::InvalidInput(
            "Corrupt exploration board retained".into(),
        ));
    }
    Ok(value.clone())
}
pub fn empty() -> Value {
    json!({"version":1,"updatedAt":0,"findings":[]})
}
pub fn prior(value: &Value) -> Result<String, DomainError> {
    let value = board(value)?;
    let rows = value["findings"].as_array().unwrap();
    if rows.is_empty() {
        return Ok("(empty)".into());
    }
    Ok(rows
        .iter()
        .skip(rows.len().saturating_sub(8))
        .map(|r| {
            format!(
                "- {}: {}",
                r["question"].as_str().unwrap(),
                r["summary"].as_str().unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}
/// Native board presentation carries age and checked source revisions. A matching
/// file hash says nothing about the correctness or completeness of the summary.
pub fn prior_with_freshness(value: &Value, now: u64, mut inspect: impl FnMut(&str) -> String) -> Result<String, DomainError> {
    let value = board(value)?;
    let rows = value["findings"].as_array().unwrap();
    if rows.is_empty() { return Ok("(empty)".into()); }
    let mut lines = Vec::new();
    for row in rows.iter().skip(rows.len().saturating_sub(8)) {
        let age = row["at"].as_u64().filter(|at| *at <= now).map(|at| (now-at).to_string()).unwrap_or_else(|| "unknown".into());
        let sources: Vec<_> = row["sourceReferences"].as_array().into_iter().flatten().filter_map(Value::as_str).take(8).map(|reference| json!({"reference":reference,"status":inspect(reference)})).collect();
        lines.push(json!({"question":row["question"],"summary":row["summary"],"ageMs":age,"childSession":row["childSession"],"resultArtifact":row["resultArtifact"],"sources":sources,"claimStatus":"historical summary; not independently verified","sourceCoverage":if sources.is_empty(){"unknown; inspect current source"}else{"recorded reads only; inspect decisive evidence before acting"}}).to_string());
    }
    Ok(lines.join("\n"))
}
pub fn merge(questions: &[String], results: &[Value], now: u64) -> Result<Vec<Value>, DomainError> {
    if questions.is_empty() || questions.len() > 4 || questions.len() != results.len() {
        return Err(DomainError::InvalidInput("Exploration result count".into()));
    }
    questions.iter().zip(results).map(|(q,r)| {
        bounded_json(r)?;
        if q.trim().is_empty() || r["type"] != "explore" || !r["state"].is_string() || !r["summary"].is_string() {
            return Err(DomainError::InvalidInput("Exploration result".into()));
        }
        let evidence = r["evidence"].as_array().ok_or(DomainError::Conflict)?;
        let evidence: Vec<String> = evidence.iter().skip(evidence.len().saturating_sub(8)).map(|e| {
            let text = |k:&str| e[k].as_str().unwrap_or("");
            match text("kind") {
                "bash" => format!("bash: {} → {}", text("command"),e["exitCode"].as_i64().map(|n|n.to_string()).unwrap_or_else(||"killed".into())),
                "file" => format!("{}: {}", text("action"),text("path")),
                "web" => format!("fetch: {}", text("url")),
                kind => kind.into(),
            }
        }).collect();
        let summary = r["summary"].as_str().unwrap().trim();
        let mut finding = json!({"question":q,"summary":if summary.is_empty(){"(no output)"}else{summary},"evidence":evidence,"at":now,"state":r["state"]});
        for key in ["childSession", "checkpointId", "sourceReferences"] {
            if let Some(value) = r.get(key) { finding[key] = value.clone(); }
        }
        Ok(finding)
    }).collect()
}
pub fn append(value: &Value, findings: &[Value], now: u64) -> Result<Value, DomainError> {
    let mut value = board(value)?;
    let rows = value["findings"].as_array_mut().unwrap();
    rows.extend_from_slice(findings);
    rows.drain(..rows.len().saturating_sub(12));
    value["updatedAt"] = json!(now);
    board(&value)
}
pub fn report(findings: &[Value]) -> Result<String, DomainError> {
    board(&json!({"version":1,"findings":findings}))?;
    let mut lines = vec![format!(
        "Explore report ({} worker{}):",
        findings.len(),
        if findings.len() == 1 { "" } else { "s" }
    )];
    for (i, f) in findings.iter().enumerate() {
        lines.extend([
            String::new(),
            format!(
                "{}. {} [{}]",
                i + 1,
                f["question"].as_str().unwrap(),
                f["state"].as_str().unwrap_or("unknown")
            ),
            f["summary"].as_str().unwrap().into(),
        ]);
        if let Some(evidence) = f["evidence"].as_array().filter(|a| !a.is_empty()) {
            lines.push("Evidence:".into());
            lines.extend(
                evidence
                    .iter()
                    .map(|e| format!("- {}", e.as_str().unwrap_or(""))),
            );
        }
    }
    Ok(lines.join("\n"))
}
pub fn validate_rollup(data: &Value, args: &Value) -> Result<(), DomainError> {
    bounded_json(data)?;
    let questions = questions(args)?;
    let results = data["results"].as_array().ok_or(DomainError::Conflict)?;
    let at = data["at"].as_u64().ok_or(DomainError::Conflict)?;
    if data["findings"] != json!(merge(&questions, results, at)?) {
        return Err(DomainError::Conflict);
    }
    Ok(())
}
