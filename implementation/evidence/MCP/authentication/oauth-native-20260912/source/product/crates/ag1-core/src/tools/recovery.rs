//! Session-owned missing-tool recovery. P10 observes each newly completed native
//! result once; replayed receipts must not increment counters. Guidance is a
//! model-facing contribution after raw custody, never verification evidence.
use crate::DomainError;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const SHELL_RECOVERY: &str = concat!(
    "Shell recovery: the bash tool is available, but the requested executable or subcommand does not exist. ",
    "Do not invent lifecycle/completion shell commands such as ack, end, checkpoint, end-task, or phase-close. ",
    "If the requested work has already passed its verifier, stop calling tools and provide the final answer now. ",
    "Otherwise use only an executable confirmed from the workspace or PATH."
);

#[derive(Default)]
pub struct FailureTracker {
    failures: BTreeMap<String, u64>,
    order: Vec<String>,
    notified: BTreeSet<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub tool_name: String,
    pub failure_count: u64,
    pub corrective_message: String,
}
pub struct PromptGuidance {
    pub content: String,
    pub details: String,
    pub warnings: Vec<Warning>,
}
impl FailureTracker {
    /// Match the SDK after-tool classification without copying or lowercasing a
    /// potentially large producer spool. Unrelated failures do not clear history.
    pub fn observe(
        &mut self,
        tool: &str,
        is_error: bool,
        text: &str,
    ) -> Result<Option<&'static str>, DomainError> {
        if tool.is_empty() || tool.len() > 128 || tool.chars().any(char::is_control) {
            return Err(DomainError::InvalidInput("failure tool name".into()));
        }
        if text.len() > 128 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("failure result text".into()));
        }
        if !is_error {
            self.failures.remove(tool);
            self.order.retain(|name| name != tool);
            self.notified.remove(tool);
            return Ok(None);
        }
        if !contains(text, b"not found") {
            return Ok(None);
        }
        if self.failures.len() >= 256 && !self.failures.contains_key(tool) {
            return Err(DomainError::LimitExceeded("failure tool inventory".into()));
        }
        if !self.failures.contains_key(tool) {
            self.order.push(tool.into());
        }
        let count = self.failures.entry(tool.into()).or_default();
        *count = count.saturating_add(1);
        Ok((tool == "bash"
            && *count >= 2
            && (contains(text, b"command not found") || contains(text, b"exited with code 127")))
        .then_some(SHELL_RECOVERY))
    }

    pub fn reset(&mut self) {
        self.failures.clear();
        self.order.clear();
        self.notified.clear();
    }
    /// New parent prompt only, preserving SDK one-time notification until that
    /// tool succeeds or the host explicitly resets the working failure state.
    pub fn prepare_prompt(
        &mut self,
        active_tools: &[String],
    ) -> Result<Option<PromptGuidance>, DomainError> {
        if active_tools.len() > 256
            || active_tools.iter().any(|tool| {
                tool.is_empty() || tool.len() > 128 || tool.chars().any(char::is_control)
            })
        {
            return Err(DomainError::InvalidInput("failure active tools".into()));
        }
        let tools = self
            .order
            .iter()
            .filter(|tool| self.failures[*tool] >= 3 && !self.notified.contains(*tool))
            .cloned()
            .collect::<Vec<_>>();
        if tools.is_empty() {
            return Ok(None);
        }
        let suggestions = tools
            .iter()
            .map(|tool| {
                let failed = tool.to_lowercase();
                let prefix = failed.chars().take(4).collect::<String>();
                let similar = active_tools
                    .iter()
                    .filter(|active| {
                        let active = active.to_lowercase();
                        active.contains(&prefix)
                            || failed.contains(&active.chars().take(4).collect::<String>())
                    })
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>();
                let suggestion = if similar.is_empty() {
                    String::new()
                } else {
                    format!(" Consider using: {}.", similar.join(", "))
                };
                format!(
                    "- `{tool}` failed {} times.{suggestion}",
                    self.failures[tool]
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let available = if active_tools.is_empty() {
            String::new()
        } else {
            format!("\n\nAvailable tools: {}", active_tools.join(", "))
        };
        let content = format!(
            "<tool_failure_warning>\nThe following tools have failed repeatedly with \"not found\" errors:{suggestions}{available}\n</tool_failure_warning>"
        );
        self.notified.extend(tools.iter().cloned());
        let warnings = tools
            .iter()
            .map(|tool| Warning {
                tool_name: tool.clone(),
                failure_count: self.failures[tool],
                corrective_message: content.clone(),
            })
            .collect();
        Ok(Some(PromptGuidance {
            content,
            details: format!("Tool failures detected: {}", tools.join(", ")),
            warnings,
        }))
    }
}
fn contains(text: &str, needle: &[u8]) -> bool {
    text.as_bytes()
        .windows(needle.len())
        .any(|part| part.eq_ignore_ascii_case(needle))
}
