//! Pinned request-failure, overflow and retry classification. These decisions
//! grant no replay authority: the native owner must drain and fence the request.
use crate::DomainError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;

#[derive(Clone, Debug)]
pub struct Settings {
    pub enabled: bool,
    pub max_retries: u32,
    pub base_delay_ms: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 3,
            base_delay_ms: 2000,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Configuration,
    BackendUnavailable,
    ProviderCapacity,
}
fn contains_any(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| text.contains(pattern))
}
pub fn capacity(error: &str) -> bool {
    static FRACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d+\s*/\s*\d+\b").unwrap());
    let text = error.to_lowercase();
    contains_any(
        &text,
        &[
            "resourceexhausted",
            "resource exhausted",
            "rate limit",
            "too many requests",
            "429",
        ],
    ) || FRACTION.is_match(&text)
        && contains_any(&text, &["window", "request limit", "worker", "quota"])
}
pub fn classify(message: &Value) -> Option<FailureKind> {
    let text = message["errorMessage"].as_str()?.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    let provider = message["provider"].as_str().unwrap_or("").to_lowercase();
    if [
        "local",
        "llamacpp",
        "llama.cpp",
        "lmstudio",
        "lm-studio",
        "ollama",
    ]
    .contains(&provider.as_str())
        && contains_any(
            &text,
            &[
                "connection error",
                "connection refused",
                "fetch failed",
                "socket hang up",
                "timed out",
                "timeout",
                "other side closed",
                "reset before headers",
                "lm studio",
                "upstream connect",
            ],
        )
    {
        return Some(FailureKind::BackendUnavailable);
    }
    if capacity(&text) {
        return Some(FailureKind::ProviderCapacity);
    }
    if contains_any(
        &text,
        &[
            "authentication token",
            "invalid issuer",
            "invalid api key",
            "no auth configured",
            "unauthorized",
            "forbidden",
            "oauth",
            "credential",
            "not from a valid issuer",
        ],
    ) || text.contains("api key") && !text.contains("retry")
    {
        return Some(FailureKind::Configuration);
    }
    None
}
pub fn overflow(message: &Value, window: u64) -> bool {
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r"prompt is too long",
            r"request_too_large",
            r"input is too long for requested model",
            r"exceeds the context window",
            r"input token count.*exceeds the maximum",
            r"maximum prompt length is \d+",
            r"reduce the length of the messages",
            r"maximum context length is \d+ tokens",
            r"exceeds the limit of \d+",
            r"exceeds the available context size",
            r"greater than the context length",
            r"context window exceeds limit",
            r"exceeded model token limit",
            r"too large for model with \d+ maximum context length",
            r"model_context_window_exceeded",
            r"prompt too long; exceeded (?:max )?context length",
            r"context[_ ]length[_ ]exceeded",
            r"too many tokens",
            r"token limit exceeded",
            r"^4(?:00|13)\s*(?:status code)?\s*\(no body\)",
        ]
        .iter()
        .map(|p| Regex::new(&format!("(?i){p}")).unwrap())
        .collect()
    });
    static EXCLUDED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^(Throttling error|Service unavailable):|rate limit|too many requests|tokens? per min").unwrap()
    });
    if message["stopReason"] == "error"
        && let Some(text) = message["errorMessage"].as_str()
        && !EXCLUDED.is_match(text)
        && PATTERNS.iter().any(|p| p.is_match(text))
    {
        return true;
    }
    window > 0
        && message["stopReason"] == "stop"
        && message["usage"]["input"]
            .as_u64()
            .unwrap_or(0)
            .saturating_add(message["usage"]["cacheRead"].as_u64().unwrap_or(0))
            > window
}
pub fn retryable(message: &Value, window: u64) -> bool {
    let Some(text) = message["errorMessage"].as_str().filter(|s| !s.is_empty()) else {
        return false;
    };
    if message["stopReason"] != "error"
        || classify(message).is_some_and(|kind| kind != FailureKind::ProviderCapacity)
        || overflow(message, window)
    {
        return false;
    }
    static REASON: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)Provider (?:stop_reason|finish_reason):\s*([\w-]+)").unwrap()
    });
    // P06 native transport emits the second structured form after draining the
    // HTTP body. Its status has the same precedence as the SDK's API marker.
    static STATUS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)API error \((\d{3})\)|^Provider HTTP (\d{3}):").unwrap());
    static PROSE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)overloaded|provider.?returned.?error|rate.?limit|too many requests|429|500|502|503|504|service.?unavailable|server.?error|internal.?error|network.?error|connection.?error|connection.?refused|other side closed|fetch failed|upstream.?connect|reset before headers|socket hang up|ended without|timed? out|timeout|terminated|retry delay|resourceexhausted|resource exhausted|request limit reached|worker local total request limit|degenerate stream|repetition loop").unwrap()
    });
    if let Some(reason) = REASON.captures(text) {
        return reason[1].eq_ignore_ascii_case("network_error");
    }
    if text.to_lowercase().contains("provider response status:") {
        return false;
    }
    if let Some(status) = STATUS.captures(text) {
        let status = status
            .get(1)
            .or_else(|| status.get(2))
            .unwrap()
            .as_str()
            .parse::<u16>()
            .unwrap();
        return status == 429 || status >= 500;
    }
    PROSE.is_match(text)
}
pub fn backoff_ms(error: &str, attempt: u32, base: u64) -> Result<u64, DomainError> {
    static FRACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\s*/\s*(\d+)").unwrap());
    let exponential = base.saturating_mul(
        1u64.checked_shl(attempt.saturating_sub(1))
            .unwrap_or(u64::MAX),
    );
    let text = error.to_lowercase();
    let window = FRACTION
        .captures(&text)
        .and_then(|c| Some((c[1].parse::<u64>().ok()?, c[2].parse::<u64>().ok()?)))
        .filter(|(_, limit)| *limit > 0)
        .filter(|_| {
            contains_any(
                &text,
                &[
                    "window",
                    "request limit",
                    "worker",
                    "quota",
                    "resourceexhausted",
                    "resource exhausted",
                ],
            )
        });
    let delay = if let Some((used, limit)) = window.filter(|(used, limit)| used >= limit) {
        exponential
            .max(15000u64.saturating_mul(used.saturating_sub(limit).saturating_add(1)))
            .min(60000)
    } else if capacity(error) {
        exponential.clamp(8000, 60000)
    } else {
        exponential
    };
    if delay > 86400000 {
        return Err(DomainError::LimitExceeded("recovery delay".into()));
    }
    Ok(delay)
}
