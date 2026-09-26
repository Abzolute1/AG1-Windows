//! Mode-compatible coding completion assessment. Inputs must be acknowledged
//! native history/host observations; renderer claims grant no verification.
use crate::DomainError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};
mod ledger;
pub use ledger::{Ledger, LedgerAssessment, Observation};

macro_rules! pattern {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pattern).unwrap());
    };
}
pattern!(
    TEST,
    r"(?i)(?:^|[;&|]\s*|\s)(?:(?:python|python\d+(?:\.\d+)?)\s+-m\s+)?pytest(?:\s|$)|\b(?:npm|pnpm|yarn|bun)\s+(?:run\s+)?test(?:\s|$)|(?:^|\s)(?:\./mvnw|\./gradlew)\s+test(?:\s|$)|\b(?:node\s+--test|go\s+test|cargo\s+test|dotnet\s+test|mvn\s+test|gradle\s+test|defects4j\s+test)(?:\s|$)|\b(?:ant|\./ant)\b[^\n;&|]*\btest(?:class)?\b|\bjava\b[^\n;&|]*\borg\.junit\.runner\.JUnitCore\b"
);
pattern!(
    NO_TESTS,
    r"(?i)\b(?:no tests? (?:ran|collected)|collected 0 items?|0 tests?)\b"
);
// npm's script banner can end in `@1.0.0 test`; that is package
// metadata, not a "0 test" count. Preserve all other no-test detection.
pattern!(NPM_SCRIPT_HEADER, r"(?m)^[ \t]*(?:#[ \t]*)?>[ \t]+\S+@\S+[ \t]+test(?:[ \t][^\r\n]*)?$");
pattern!(
    ONLY_SKIPPED,
    r"(?i)\bcollected 0 items?\s*/\s*[1-9]\d* skipped\b"
);
pattern!(
    FAILURE,
    r"(?i)Command exited with code [1-9]\d*|Command timed out|Command was killed by a signal"
);
pattern!(
    FAILURE_SUMMARY,
    r"(?i)\b[1-9]\d* failed\b|\bFailing tests:\s*[1-9]\d*\b"
);
pattern!(
    SAFETY,
    r"(?i)\b(?:Blocking bash access|blocked by (?:the )?safety|permission denied)\b"
);
pattern!(
    USAGE,
    r"(?i)\b(?:unrecognized arguments?|unknown option|usage:\s*(?:pytest|py\.test))\b"
);
pattern!(
    SHELL_ERROR,
    r"(?i)(?:^|\n)(?:/bin/)?(?:ba|da|z)?sh:\s*(?:line\s+\d+:\s*)?.*\b(?:command not found|No such file or directory)\b"
);
pattern!(
    INSTALL,
    r"(?i)\b(?:pip|uv|npm|pnpm|yarn|bun)\s+(?:install|add)\b"
);
pattern!(
    INSTALL_ERROR,
    r"(?i)\b(?:Could not find a version|No matching distribution|network (?:is )?unreachable|failed to (?:download|fetch|install))\b"
);
pattern!(
    BASELINE,
    r"(?i)\bgit\s+(?:stash|diff\s+--quiet|restore|checkout)\b"
);
pattern!(FAILED_PREFIX, r"(?:^|\n)FAILED\s+([^\s]+)");
pattern!(
    FAILED_SUFFIX,
    r"(?:^|\n)[- ]*([^\s]+::[^\s]+)\s+FAILED(?:\s|$)"
);
pattern!(
    PASSED_SUFFIX,
    r"(?:^|\n)[- ]*([^\s]+::[^\s]+)\s+PASSED(?:\s|$)"
);
pattern!(SEPARATOR, r"\|\||&&|[;|]");
pattern!(REDIRECT, r"\s+(?:2?>&?\d|>[^\s]+).*$");
pattern!(SPACE, r"\s+");
pattern!(ROLE_PREFIX, r"(?i)^<\|(?:im_start|start_header_id)\|>");
pattern!(ROLE_SUFFIX, r"(?i)<\|(?:im_end|end_header_id|eot_id)\|>$");
pattern!(
    PROMISE,
    r"(?i)(?:^|[\n.!?]\s*)(?:(?:good|okay|now|next)[,.:]?\s+)*(?:(?:let me|i\s+(?:need|will|must|should|am going)\s+to)\s+)?(?:add|apply|build|check|continue|create|edit|finish|fix|implement|inspect|make|read|remove|repair|run|test|update|verify|wire|write)\b[^.!?]*(?::|\.\s*)?$"
);

/// Shared pinned controlled-task intent classifier; this only decides whether
/// completion evidence is required, never whether a tool is authorized.
pub fn expects_mutation(prompt: &str) -> bool {
    super::policy::classify(prompt).mutation_expected()
}
pub fn looks_final(message: &Value) -> bool {
    if matches!(message["stopReason"].as_str(), Some("error" | "aborted")) {
        return true;
    }
    if blocks(message).any(|b| b["type"] == "toolCall") {
        return false;
    }
    let text = blocks(message)
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    !text.trim().is_empty() && !is_role_sentinel(&text)
}

pub fn is_test_command(command: &str) -> bool {
    TEST.is_match(command)
}
pub fn normalize_test_command(command: &str) -> String {
    let start = TEST.find(command).map_or(0, |m| m.start());
    let command = SEPARATOR
        .split(&command[start..])
        .next()
        .unwrap_or("")
        .trim_start_matches(|ch: char| ch.is_whitespace() || ";&|".contains(ch));
    let command = REDIRECT.replace(command, "");
    SPACE.replace_all(&command, " ").trim().into()
}
fn checks(output: &str, patterns: &[&Regex]) -> Result<Vec<String>, DomainError> {
    let mut checks = BTreeSet::new();
    for pattern in patterns {
        for capture in pattern.captures_iter(output) {
            if capture[1].len() > 4096 || checks.len() >= 4096 && !checks.contains(&capture[1]) {
                return Err(DomainError::LimitExceeded(
                    "verification check identities".into(),
                ));
            }
            checks.insert(capture[1].to_owned());
        }
    }
    let mut checks: Vec<_> = checks.into_iter().collect();
    checks.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    Ok(checks)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    NoTests,
    Inconclusive,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Verification {
    pub command_key: String,
    pub status: Status,
    pub failed_checks: Vec<String>,
    pub passed_checks: Vec<String>,
    pub baseline_probe: bool,
}
pub fn classify(command: &str, output: &str, is_error: bool) -> Result<Verification, DomainError> {
    if command.len() > 65536 || output.len() > 128 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("verification input".into()));
    }
    let failed_checks = checks(output, &[&FAILED_PREFIX, &FAILED_SUFFIX])?;
    let passed_checks = checks(output, &[&PASSED_SUFFIX])?;
    let prefix = &command[..TEST.find(command).map_or(0, |m| m.start())];
    let failed_before = INSTALL.is_match(prefix) && INSTALL_ERROR.is_match(output);
    let diagnostic = NPM_SCRIPT_HEADER.replace_all(output, "");
    let no_tests = NO_TESTS.is_match(&diagnostic);
    let status = if no_tests && ONLY_SKIPPED.is_match(output) {
        Status::Inconclusive
    } else if no_tests {
        Status::NoTests
    } else if SAFETY.is_match(output)
        || USAGE.is_match(output)
        || failed_before
        || SHELL_ERROR.is_match(output)
    {
        Status::Inconclusive
    } else if FAILURE_SUMMARY.is_match(output) {
        Status::Failed
    } else if is_error || FAILURE.is_match(output) {
        if command.contains('|') && failed_checks.is_empty() {
            Status::Inconclusive
        } else {
            Status::Failed
        }
    } else {
        Status::Passed
    };
    Ok(Verification {
        command_key: normalize_test_command(command),
        status,
        failed_checks,
        passed_checks,
        baseline_probe: BASELINE.is_match(command),
    })
}
fn blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b.is_object())
}
fn text(message: &Value) -> String {
    blocks(message)
        .map(|block| {
            if block["type"] == "text" {
                block["text"].as_str().unwrap_or("")
            } else {
                ""
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn is_role_sentinel(text: &str) -> bool {
    let prefix = ROLE_PREFIX.replace(text.trim(), "");
    let suffix = ROLE_SUFFIX.replace(&prefix, "");
    matches!(
        suffix.trim().to_ascii_lowercase().as_str(),
        "user" | "assistant" | "system"
    )
}
pub fn needs_continuation(message: &Value) -> bool {
    if message["role"] != "assistant" || blocks(message).any(|b| b["type"] == "toolCall") {
        return false;
    }
    let text = blocks(message)
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    let thinking = blocks(message).any(|b| {
        b["type"] == "thinking" && b["thinking"].as_str().is_some_and(|s| !s.trim().is_empty())
    });
    if text.is_empty() || is_role_sentinel(text) {
        return thinking
            && (matches!(
                message["stopReason"].as_str(),
                Some("length" | "max_tokens")
            ) || message["stopReason"] == "stop" && is_role_sentinel(text));
    }
    message["stopReason"] == "stop" && text.encode_utf16().count() <= 800 && PROMISE.is_match(text)
}
#[derive(Clone, Copy)]
pub enum VerificationAuthority {
    MessageHistory,
    TypedLedger,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    pub passed: bool,
    pub reasons: Vec<String>,
    pub latest_mutation_index: Option<usize>,
    pub passing_verification_count: usize,
    pub failing_verification_count: usize,
    pub no_tests_collected_count: usize,
    pub untracked_paths: Vec<String>,
    pub modified_test_paths: Vec<String>,
}
pub fn assess(
    messages: &[Value],
    untracked: &[String],
    modified_tests: &[String],
    authority: VerificationAuthority,
) -> Result<Assessment, DomainError> {
    if messages.len() > 100_000
        || untracked.len() > 4096
        || modified_tests.len() > 4096
        || untracked
            .iter()
            .chain(modified_tests)
            .any(|path| path.len() > 4096)
    {
        return Err(DomainError::LimitExceeded(
            "completion history or paths".into(),
        ));
    }
    let mut commands = BTreeMap::new();
    let mut latest = None;
    for (index, message) in messages.iter().enumerate() {
        if message["role"] == "assistant" {
            for block in blocks(message).filter(|b| b["type"] == "toolCall" && b["name"] == "bash")
            {
                if let (Some(id), Some(command)) =
                    (block["id"].as_str(), block["arguments"]["command"].as_str())
                {
                    commands.insert(id, command);
                }
            }
        }
        if message["role"] == "toolResult"
            && message["isError"] != true
            && matches!(
                message["toolName"].as_str(),
                Some(
                    "edit"
                        | "write"
                        | "append"
                        | "notebook_edit"
                        | "NotebookEdit"
                        | "patch"
                        | "multi_edit"
                )
            )
        {
            latest = Some(index);
        }
    }
    let mut verifications = BTreeMap::new();
    for message in &messages[latest.map_or(messages.len(), |index| index + 1)..] {
        if message["role"] != "toolResult" || message["toolName"] != "bash" {
            continue;
        }
        let Some(command) = message["toolCallId"]
            .as_str()
            .and_then(|id| commands.get(id))
        else {
            continue;
        };
        if !is_test_command(command) {
            continue;
        }
        let verification = classify(command, &text(message), message["isError"] == true)?;
        verifications.insert(verification.command_key.clone(), verification);
    }
    let count = |status| {
        verifications
            .values()
            .filter(|v| v.status == status)
            .count()
    };
    let passing = count(Status::Passed);
    let failing = count(Status::Failed);
    let empty = count(Status::NoTests);
    let mut reasons = vec![];
    if latest.is_none() {
        reasons.push("no successful source mutation was recorded".into());
    }
    if matches!(authority, VerificationAuthority::MessageHistory) {
        if passing == 0 {
            reasons
                .push("no successful test command was recorded after the latest mutation".into());
        }
        if failing > 0 {
            reasons.push(format!(
                "{failing} test command(s) still fail after the latest mutation"
            ));
        }
        if empty > 0 {
            reasons.push(format!(
                "{empty} command(s) collected no tests after the latest mutation"
            ));
        }
    }
    if !untracked.is_empty() {
        reasons.push(format!(
            "untracked workspace paths remain: {}",
            untracked.join(", ")
        ));
    }
    if !modified_tests.is_empty() {
        reasons.push(format!(
            "repository test paths were modified: {}",
            modified_tests.join(", ")
        ));
    }
    if messages
        .iter()
        .rev()
        .find(|m| m["role"] == "assistant")
        .is_some_and(needs_continuation)
    {
        reasons.push("the last assistant response was truncated during internal reasoning".into());
    }
    Ok(Assessment {
        passed: reasons.is_empty(),
        reasons,
        latest_mutation_index: latest,
        passing_verification_count: passing,
        failing_verification_count: failing,
        no_tests_collected_count: empty,
        untracked_paths: untracked.into(),
        modified_test_paths: modified_tests.into(),
    })
}
