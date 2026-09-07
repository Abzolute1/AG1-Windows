//! Delegated-task policy and checkpoint state. Execution is an isolated host
//! loop; these definitions never authorize a shell command or recurse into task.
use crate::{
    DomainError,
    context::hygiene::HygieneOptions,
    model::CapabilityProfile,
    tools::policy::{Mode, Policy},
};
use serde::{Deserialize, Serialize};
pub mod checkpoint;
pub mod routing;
pub mod tool;
pub mod tracking;

pub const DEFAULT_MAX_TURNS: u64 = 12;
pub const MAX_TURNS: u64 = 25;
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 300;
pub const DEFAULT_TOKEN_BUDGET: u64 = 200_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Explore,
    Research,
    Implement,
    Verify,
    Plan,
}
impl Profile {
    pub fn instructions(self) -> &'static str {
        match self {
            Self::Explore => {
                "Inspect the workspace and answer with evidence. Use read-only tools. Cite file:line for code-derived claims and state what you could not determine."
            }
            Self::Research => {
                "Answer using the workspace and web. Search for sources, fetch the strongest URLs, and cite URLs and file:line for derived claims."
            }
            Self::Implement => {
                "Make the requested changes and verify them. Stay scoped to the task. Report changed paths, verification commands and exit codes, and anything left undone."
            }
            Self::Verify => {
                "Run the requested checks and report each command, exit code, verdict, and relevant output. Do not change files to make checks pass; report failures as findings."
            }
            Self::Plan => {
                "Inspect the workspace as needed. Return a numbered actionable plan grounded in real paths, with verification for each step."
            }
        }
    }
    /// Declared source tool surface. Hosts must intersect actual capabilities;
    /// readonly bash requires separate enforcement, never this name list alone.
    pub fn tools(self) -> Vec<&'static str> {
        let mut tools = vec!["read", "grep", "find", "ls", "repo", "bash"];
        match self {
            Self::Explore | Self::Plan => (),
            Self::Research => tools.extend(["webSearch", "webFetch"]),
            Self::Verify => tools.push("check"),
            Self::Implement => tools.extend([
                "edit",
                "write",
                "memory",
                "job",
                "check",
                "screenshot_url",
                "interact_then_screenshot",
                "verify_ui",
                "webSearch",
                "webFetch",
            ]),
        }
        tools
    }
    pub fn readonly(self) -> bool {
        matches!(self, Self::Explore | Self::Research | Self::Plan)
    }
    pub fn policy(self, parent: &Policy) -> Policy {
        let mut policy = parent.clone();
        if matches!(self, Self::Implement | Self::Verify) && policy.mode == Mode::Headless {
            policy.mode = Mode::Autonomous;
        }
        policy
    }
    pub fn uses_parent_model(self) -> bool {
        matches!(self, Self::Implement | Self::Plan)
    }
}

pub fn max_turns(requested: Option<u64>) -> u64 {
    requested
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_TURNS)
        .min(MAX_TURNS)
}
/// Source subagents use lighter request-only hygiene, not the parent's full CV.
pub fn hygiene(profile: &CapabilityProfile) -> HygieneOptions {
    if profile.recommended_compaction_policy == "local_small" {
        HygieneOptions {
            keep_turns: 1,
            max_tool_result_chars: 160,
            max_hidden_custom_chars: 100,
        }
    } else if profile.tool_call_reliability == "medium" {
        HygieneOptions {
            keep_turns: 2,
            max_tool_result_chars: 180,
            max_hidden_custom_chars: 140,
        }
    } else {
        HygieneOptions::default()
    }
}
pub fn checkpoint_id(id: &str) -> Result<(), DomainError> {
    if id.len() > 128
        || !id.starts_with("ckpt_")
        || id.len() <= 5
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(DomainError::InvalidInput("checkpoint identity".into()));
    }
    Ok(())
}
