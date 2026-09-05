//! Session-owned missing-tool recovery. P10 observes each newly completed native
//! result once; replayed receipts must not increment counters. Guidance is a
//! separate system contribution, never verification evidence or execution policy.
use crate::DomainError;
use std::collections::BTreeMap;

pub const SHELL_RECOVERY: &str = concat!(
    "Shell recovery: the bash tool is available, but the requested executable or subcommand does not exist. ",
    "Do not invent lifecycle/completion shell commands such as ack, end, checkpoint, end-task, or phase-close. ",
    "If the requested work has already passed its verifier, stop calling tools and provide the final answer now. ",
    "Otherwise use only an executable confirmed from the workspace or PATH."
);

#[derive(Default)]
pub struct FailureTracker {
    failures: BTreeMap<String, u64>,
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
            return Ok(None);
        }
        if !contains(text, b"not found") {
            return Ok(None);
        }
        if self.failures.len() >= 256 && !self.failures.contains_key(tool) {
            return Err(DomainError::LimitExceeded("failure tool inventory".into()));
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
    }
}
fn contains(text: &str, needle: &[u8]) -> bool {
    text.as_bytes()
        .windows(needle.len())
        .any(|part| part.eq_ignore_ascii_case(needle))
}
