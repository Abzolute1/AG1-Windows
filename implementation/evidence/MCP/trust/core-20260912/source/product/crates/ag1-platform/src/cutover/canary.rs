//! Sequential per-model acceptance. Rehearsal reports are never production
//! authority tokens. The live adapter owns its runtime throughout the matrix.
use ag1_core::{DomainError, PortFuture};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub mod native;
type Result<T> = std::result::Result<T, DomainError>;
pub const CASES: [&str; 6] = [
    "exact-copy",
    "reasoning",
    "tool-call",
    "tool-result",
    "context-limit",
    "cancellation",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runner {
    Typescript,
    Rust,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Request {
    pub case: String,
    pub model: Value,
    pub context: Value,
    pub options: Value,
    pub cancel_after_first_content: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Answer {
    pub message: Value,
    pub payload: Value,
    pub first_content_ms: Option<u64>,
    pub elapsed_ms: u64,
    pub cancelled: bool,
    pub settled: bool,
    #[serde(default)]
    pub runtime_before: Option<Value>,
    #[serde(default)]
    pub runtime_after: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub model: String,
    pub runner: Runner,
    pub case: String,
    pub payload_sha256: String,
    pub answer: Answer,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub version: u32,
    pub fixture: bool,
    pub status: String,
    pub rows: Vec<Row>,
    pub models: Vec<String>,
    pub failures: Vec<String>,
    pub cold_start_per_runner: bool,
    pub runtime_settled: bool,
}

/// Recheck retained evidence using the same request builder and quality rules
/// as execution. This is read-only validation, never a runtime ownership proof.
pub fn verify_recording(report: &Report, models: &[Value]) -> Result<()> {
    let ids: Vec<_> = models
        .iter()
        .map(|m| m["id"].as_str().ok_or(DomainError::Conflict))
        .collect::<Result<_>>()?;
    if report.version != 1
        || report.status != "passed"
        || !report.failures.is_empty()
        || !report.runtime_settled
        || !report.cold_start_per_runner
        || models.len() != 3
        || ids.iter().collect::<BTreeSet<_>>().len() != 3
        || report.models != ids
        || report.rows.len() != models.len() * 2 * CASES.len()
    {
        return Err(DomainError::Conflict);
    }
    let mut rows = report.rows.iter();
    for model in models {
        for runner in [Runner::Typescript, Runner::Rust] {
            let mut tool = None;
            let mut owned_pid = None;
            for case in CASES {
                let row = rows.next().ok_or(DomainError::Conflict)?;
                if row.model != model["id"].as_str().ok_or(DomainError::Conflict)?
                    || row.runner != runner
                    || row.case != case
                {
                    return Err(DomainError::Conflict);
                }
                let request = request(model, case, tool.as_ref())?;
                valid(&request, &row.answer)?;
                if row.payload_sha256
                    != super::digest(&serde_json::to_vec(&row.answer.payload).map_err(super::json)?)
                    || (case != "context-limit" && row.answer.first_content_ms.is_none())
                    || row
                        .answer
                        .first_content_ms
                        .is_some_and(|first| first > row.answer.elapsed_ms)
                {
                    return Err(DomainError::Conflict);
                }
                if !report.fixture {
                    let before = resource_sample(row.answer.runtime_before.as_ref())?;
                    if owned_pid.is_some_and(|pid| pid != before) {
                        return Err(DomainError::Conflict);
                    }
                    owned_pid = Some(before);
                    if case != "cancellation"
                        && resource_sample(row.answer.runtime_after.as_ref())? != before
                    {
                        return Err(DomainError::Conflict);
                    }
                }
                if case == "tool-call" {
                    tool = Some(row.answer.message.clone());
                }
            }
        }
    }
    Ok(())
}

fn resource_sample(value: Option<&Value>) -> Result<u64> {
    let value = value.ok_or(DomainError::Conflict)?;
    let pid = value["owned_pid"]
        .as_u64()
        .filter(|p| *p > 0)
        .ok_or(DomainError::Conflict)?;
    let gpu = &value["gpu"];
    let total = gpu["total_mib"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or(DomainError::Conflict)?;
    if value["resident_kib"]
        .as_u64()
        .filter(|rss| *rss > 0)
        .is_none()
        || gpu["used_mib"].as_u64().is_none_or(|used| used > total)
        || gpu["free_mib"].as_u64().is_none_or(|free| free > total)
    {
        return Err(DomainError::Conflict);
    }
    Ok(pid)
}

pub fn verify_recording_file(path: &std::path::Path) -> Result<Report> {
    #[derive(Deserialize)]
    struct Recording {
        report: Report,
        models: Vec<Value>,
    }
    let bytes = super::bounded(path, 128 * 1024 * 1024)?;
    let recording: Recording = serde_json::from_slice(&bytes).map_err(super::json)?;
    verify_recording(&recording.report, &recording.models)?;
    Ok(recording.report)
}

pub fn recording_digest(report: &Report) -> Result<String> {
    Ok(super::digest(
        &serde_json::to_vec(report).map_err(super::json)?,
    ))
}
/// Trusted fixture/host adapter. Public callers can obtain rehearsal reports;
/// only the concrete native adapter can establish production runtime custody.
pub trait Driver {
    fn reset(&mut self) -> PortFuture<'_, ()>;
    fn run<'a>(&'a mut self, runner: Runner, request: &'a Request) -> PortFuture<'a, Answer>;
}
pub fn payload(request: &Request) -> Result<Value> {
    crate::provider::requests::build_payload(&crate::provider::requests::RequestContext {
        model: &request.model,
        context: &request.context,
        options: &request.options,
        anthropic_oauth: false,
        google_project: None,
        request_id: "p13-canary",
    })
}
fn text(message: &Value) -> String {
    message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}
fn valid(request: &Request, answer: &Answer) -> Result<()> {
    if answer.payload != payload(request)? || !answer.settled {
        return Err(DomainError::Conflict);
    }
    let message = &answer.message;
    let stop = message["stopReason"].as_str().unwrap_or("");
    let content = message["content"].as_array().ok_or(DomainError::Conflict)?;
    let okay = match request.case.as_str() {
        "exact-copy" => stop == "stop" && text(message).trim() == "P13_COPY_7e2b: αβ 0192837465",
        "reasoning" => {
            stop == "stop"
                && text(message).trim() == "703"
                && content.iter().any(|b| {
                    b["type"] == "thinking"
                        && b["thinking"].as_str().is_some_and(|s| !s.trim().is_empty())
                })
        }
        "tool-call" => {
            stop == "toolUse"
                && content.iter().filter(|b| b["type"] == "toolCall").count() == 1
                && content.iter().any(|b| {
                    b["type"] == "toolCall"
                        && b["name"] == "p13_echo"
                        && b["arguments"] == json!({"value":"P13_TOOL_7"})
                })
        }
        "tool-result" => stop == "stop" && text(message).trim() == "P13_TOOL_7",
        "context-limit" => {
            let error = message["errorMessage"]
                .as_str()
                .unwrap_or("")
                .to_lowercase();
            stop == "error"
                && error.contains("context")
                && ["exceed", "too long", "too large", "maximum", "larger"]
                    .iter()
                    .any(|word| error.contains(word))
        }
        "cancellation" => {
            stop == "aborted" && answer.cancelled && answer.first_content_ms.is_some()
        }
        _ => false,
    };
    if !okay || (request.case != "cancellation" && answer.cancelled) {
        return Err(DomainError::Unavailable(format!(
            "P13 {} acceptance failed",
            request.case
        )));
    }
    Ok(())
}
fn request(model: &Value, case: &str, tool_message: Option<&Value>) -> Result<Request> {
    let limit = model["contextWindow"]
        .as_u64()
        .ok_or(DomainError::Conflict)?;
    if !(1024..=262144).contains(&limit) || model["maxTokens"].as_u64().unwrap_or(0) < 1024 {
        return Err(DomainError::InvalidInput("Canary model limits".into()));
    }
    let prompt = match case {
        "exact-copy" => "Reply with exactly this text and nothing else: P13_COPY_7e2b: αβ 0192837465".into(),
        "reasoning" => "What is 37 times 19? Reply with only the decimal integer and no explanation.".into(),
        "tool-call" | "tool-result" => "Call p13_echo with value P13_TOOL_7. After its result, reply with exactly the returned value and nothing else.".into(),
        "context-limit" => " p13".repeat(limit as usize + 2048),
        "cancellation" => "Count upward starting at 1, one integer per line. Continue until your output limit.".into(),
        _ => return Err(DomainError::Conflict),
    };
    let mut context = json!({
        "systemPrompt":"You are helping verify a local agent migration. The user requests harmless tests of copying text, arithmetic, and the provided echo tool. Complete each requested test and follow its output format. For an exact-output request, the requested value is the whole response. Include no explanation, note, follow-up offer, or markup.",
        "messages":[{"role":"user","content":prompt,"timestamp":0}]
    });
    if case.starts_with("tool-") {
        context["tools"] = json!([{"name":"p13_echo","description":"Return the supplied canary value unchanged.","parameters":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}}]);
        if let Some(message) = tool_message {
            let call = message["content"]
                .as_array()
                .and_then(|c| c.iter().find(|b| b["type"] == "toolCall"))
                .ok_or(DomainError::Conflict)?;
            let messages = context["messages"].as_array_mut().unwrap();
            messages.push(message.clone());
            // Pure tool execution; no workspace file or external effect.
            messages.push(json!({"role":"toolResult","toolCallId":call["id"],"toolName":"p13_echo","content":[{"type":"text","text":call["arguments"]["value"]}],"isError":false,"timestamp":0}));
        }
    }
    let mut options = json!({"maxTokens":1024});
    if case == "tool-call" {
        // This case verifies tool-call transport. Require the advertised tool
        // instead of letting a model shortcut an echo task with plain text.
        options["toolChoice"] = json!("required");
    }
    if case == "reasoning" {
        // A high-reasoning request must have the captured model's output
        // allowance: a small generic probe cap can truncate thinking before
        // its answer. Both adapters receive this same unchanged model limit.
        options["maxTokens"] = model["maxTokens"].clone();
        options["reasoningEffort"] = json!("high");
    }
    Ok(Request {
        case: case.into(),
        model: model.clone(),
        context,
        options,
        cancel_after_first_content: case == "cancellation",
    })
}
pub async fn rehearse(driver: &mut impl Driver, models: &[Value]) -> Report {
    matrix(driver, models, true).await
}
async fn matrix(driver: &mut impl Driver, models: &[Value], fixture: bool) -> Report {
    let mut report = Report {
        version: 1,
        fixture,
        status: "failed".into(),
        rows: vec![],
        models: vec![],
        failures: vec![],
        cold_start_per_runner: true,
        runtime_settled: false,
    };
    let mut seen = BTreeSet::new();
    let outcome: Result<()> = async {
        if models.len() != 3 {
            return Err(DomainError::InvalidInput(
                "All three configured models are required".into(),
            ));
        }
        for model in models {
            let id = model["id"].as_str().ok_or(DomainError::Conflict)?;
            if !seen.insert(id.to_owned()) {
                return Err(DomainError::Conflict);
            }
            report.models.push(id.into());
            for runner in [Runner::Typescript, Runner::Rust] {
                driver.reset().await?;
                let mut tool = None;
                for case in CASES {
                    let request = request(model, case, tool.as_ref())?;
                    let answer = driver.run(runner, &request).await?;
                    // Retain failed quality responses as evidence, never cherry-pick.
                    report.rows.push(Row {
                        model: id.into(),
                        runner,
                        case: case.into(),
                        payload_sha256: super::digest(
                            &serde_json::to_vec(&answer.payload).map_err(super::json)?,
                        ),
                        answer: answer.clone(),
                    });
                    valid(&request, &answer)?;
                    if case == "tool-call" {
                        tool = Some(answer.message);
                    }
                }
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = outcome {
        report.failures.push(format!("{error:?}"));
    }
    match driver.reset().await {
        Ok(()) => report.runtime_settled = true,
        Err(error) => report
            .failures
            .push(format!("Owned runtime settlement: {error:?}")),
    }
    if report.failures.is_empty() && report.rows.len() == 36 {
        report.status = "passed".into();
    }
    report
}
