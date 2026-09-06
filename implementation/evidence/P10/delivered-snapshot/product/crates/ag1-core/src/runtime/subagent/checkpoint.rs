//! Acknowledged checkpoints stored below the UI. Failed writes leave the last
//! acknowledged state intact and require reconciliation before further updates.
use super::{Profile, checkpoint_id};
use crate::{DomainError, context::slice_utf16, storage::StoragePort};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Evidence {
    Bash {
        command: String,
        #[serde(rename = "exitCode")]
        exit_code: Option<i64>,
    },
    File {
        action: String,
        path: String,
    },
    Inspect {
        tool: String,
        detail: String,
    },
    Web {
        url: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceEntry {
    #[serde(flatten)]
    pub evidence: Evidence,
    pub turn: u64,
    pub at: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub turn: u64,
    pub at: String,
    pub text: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub total_tokens: u64,
    pub cost: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Completed,
    Failed,
    Timeout,
    Budget,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Checkpoint {
    pub id: String,
    #[serde(rename = "type")]
    pub profile: Profile,
    pub description: String,
    pub started_at: String,
    pub updated_at: String,
    pub evidence: Vec<EvidenceEntry>,
    pub summaries: Vec<Summary>,
    pub usage: Usage,
    pub last_turn: u64,
    pub tool_calls: u64,
    pub budget_pressure: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_state: Option<State>,
}
impl Checkpoint {
    pub fn new(
        id: String,
        profile: Profile,
        description: String,
        at: String,
    ) -> Result<Self, DomainError> {
        let state = Self {
            id,
            profile,
            description,
            started_at: at.clone(),
            updated_at: at,
            evidence: vec![],
            summaries: vec![],
            usage: Usage::default(),
            last_turn: 0,
            tool_calls: 0,
            budget_pressure: "low".into(),
            final_state: None,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<(), DomainError> {
        checkpoint_id(&self.id)?;
        if self.description.len() > 16 * 1024
            || self.evidence.len() > 100
            || self.summaries.len() > 20
            || !self.usage.cost.is_finite()
            || self.usage.cost < 0.0
            || !["low", "medium", "high", "critical"].contains(&self.budget_pressure.as_str())
            || [&self.started_at, &self.updated_at]
                .iter()
                .any(|at| chrono::DateTime::parse_from_rfc3339(at).is_err())
            || self
                .evidence
                .iter()
                .any(|e| chrono::DateTime::parse_from_rfc3339(&e.at).is_err())
            || self.summaries.iter().any(|s| {
                s.text.encode_utf16().count() > 2000
                    || chrono::DateTime::parse_from_rfc3339(&s.at).is_err()
            })
        {
            return Err(DomainError::InvalidInput("checkpoint state".into()));
        }
        let bytes = serde_json::to_vec(self).map_err(|_| DomainError::Conflict)?;
        if bytes.len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded("checkpoint state".into()));
        }
        Ok(())
    }
    pub fn add_evidence(
        &mut self,
        evidence: Evidence,
        turn: u64,
        at: &str,
    ) -> Result<(), DomainError> {
        let mut next = self.clone();
        next.tool_calls = next
            .tool_calls
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        next.evidence.push(EvidenceEntry {
            evidence,
            turn,
            at: at.into(),
        });
        if next.evidence.len() > 100 {
            next.evidence.remove(0);
        }
        next.updated_at = at.into();
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn add_summary(&mut self, text: &str, turn: u64, at: &str) -> Result<(), DomainError> {
        if text.trim().is_empty() {
            return Ok(());
        }
        let mut next = self.clone();
        next.summaries.push(Summary {
            text: slice_utf16(text, 2000),
            turn,
            at: at.into(),
        });
        if next.summaries.len() > 20 {
            next.summaries.remove(0);
        }
        next.updated_at = at.into();
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn resume_prompt(&self, prompt: &str) -> Result<String, DomainError> {
        self.validate()?;
        if prompt.len() > 4 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("checkpoint prompt".into()));
        }
        let profile = serde_json::to_value(self.profile).map_err(|_| DomainError::Conflict)?;
        let state = self
            .final_state
            .map(|s| serde_json::to_value(s).expect("enum"));
        let mut parts = vec![
            format!(
                "You are resuming a previous {} task that ended in state: {}.",
                profile.as_str().unwrap(),
                state.as_ref().and_then(|s| s.as_str()).unwrap_or("unknown")
            ),
            format!(
                "The previous run completed {} turns with {} tool calls.",
                self.last_turn, self.tool_calls
            ),
            String::new(),
            "## Previous findings".into(),
            String::new(),
        ];
        if !self.evidence.is_empty() {
            parts.push("Evidence collected:".into());
            for entry in &self.evidence {
                parts.push(match &entry.evidence {
                    Evidence::Bash { command, exit_code } => format!(
                        "- bash: `{command}` → exit {}",
                        exit_code
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "killed".into())
                    ),
                    Evidence::File { action, path } => format!("- {action}: {path}"),
                    Evidence::Inspect { tool, detail } => format!("- {tool}: {detail}"),
                    Evidence::Web { url } => format!("- fetch: {url}"),
                });
            }
            parts.push(String::new());
        }
        if !self.summaries.is_empty() {
            parts.push("Previous analysis progress:".into());
            for summary in self
                .summaries
                .iter()
                .skip(self.summaries.len().saturating_sub(5))
            {
                parts.push(format!(
                    "[turn {}] {}",
                    summary.turn,
                    slice_utf16(&summary.text, 500)
                ));
            }
            parts.push(String::new());
        }
        parts.extend(["## Original task".into(), prompt.into(), String::new(),
            "Continue from where the previous run left off. Prior findings are historical; revalidate source-dependent claims before acting.".into()]);
        Ok(parts.join("\n"))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub workspace: String,
    pub session: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    owner: Owner,
    checkpoint: Checkpoint,
}
pub struct Manager {
    owner: Owner,
    key: String,
    acknowledged: Checkpoint,
    revision: String,
    uncertain: bool,
}
fn key(owner: &Owner, id: &str) -> Result<String, DomainError> {
    crate::WorkspaceId::new(&owner.workspace)?;
    crate::SessionId::new(&owner.session)?;
    checkpoint_id(id)?;
    Ok(format!(
        "checkpoint-{:x}",
        Sha256::digest(json!([owner, id]).to_string().as_bytes())
    ))
}
fn bytes(owner: &Owner, checkpoint: &Checkpoint) -> Result<Vec<u8>, DomainError> {
    checkpoint.validate()?;
    serde_json::to_vec(&Stored {
        version: 1,
        owner: owner.clone(),
        checkpoint: checkpoint.clone(),
    })
    .map_err(|_| DomainError::Conflict)
}
impl Manager {
    pub fn create(
        store: &impl StoragePort,
        owner: Owner,
        checkpoint: Checkpoint,
    ) -> Result<Self, DomainError> {
        let key = key(&owner, &checkpoint.id)?;
        let revision = store.commit_metadata(
            "subagent_checkpoint",
            &key,
            None,
            &bytes(&owner, &checkpoint)?,
        )?;
        Ok(Self {
            owner,
            key,
            acknowledged: checkpoint,
            revision,
            uncertain: false,
        })
    }
    pub fn load(store: &impl StoragePort, owner: Owner, id: &str) -> Result<Self, DomainError> {
        let key = key(&owner, id)?;
        let document = store
            .load_metadata("subagent_checkpoint", &key)?
            .ok_or(DomainError::NotFound)?;
        if document.bytes.len() > 1024 * 1024 + 1024 || document.diagnostic.is_some() {
            return Err(DomainError::DurabilityUnknown);
        }
        let stored: Stored =
            serde_json::from_slice(&document.bytes).map_err(|_| DomainError::DurabilityUnknown)?;
        if stored.version != 1 || stored.owner != owner || stored.checkpoint.id != id {
            return Err(DomainError::PermissionDenied);
        }
        stored.checkpoint.validate()?;
        Ok(Self {
            owner,
            key,
            acknowledged: stored.checkpoint,
            revision: document.sha256,
            uncertain: false,
        })
    }
    pub fn acknowledged(&self) -> &Checkpoint {
        &self.acknowledged
    }
    pub fn needs_reconciliation(&self) -> bool {
        self.uncertain
    }
    pub fn commit(
        &mut self,
        store: &impl StoragePort,
        next: Checkpoint,
    ) -> Result<(), DomainError> {
        if self.uncertain {
            return Err(DomainError::DurabilityUnknown);
        }
        if next.id != self.acknowledged.id
            || next.profile != self.acknowledged.profile
            || next.started_at != self.acknowledged.started_at
            || next.description != self.acknowledged.description
            || next.last_turn < self.acknowledged.last_turn
            || next.tool_calls < self.acknowledged.tool_calls
            || next.usage.input < self.acknowledged.usage.input
            || next.usage.output < self.acknowledged.usage.output
            || next.usage.total_tokens < self.acknowledged.usage.total_tokens
            || next.usage.cost < self.acknowledged.usage.cost
            || self.acknowledged.final_state.is_some()
        {
            return Err(DomainError::Conflict);
        }
        let encoded = bytes(&self.owner, &next)?;
        match store.commit_metadata(
            "subagent_checkpoint",
            &self.key,
            Some(&self.revision),
            &encoded,
        ) {
            Ok(revision) => {
                self.acknowledged = next;
                self.revision = revision;
                Ok(())
            }
            Err(error) => {
                self.uncertain = true;
                Err(error)
            }
        }
    }
    /// Explicit read of durable state, never replay of child tools or generation.
    pub fn reconcile(&mut self, store: &impl StoragePort) -> Result<(), DomainError> {
        *self = Self::load(store, self.owner.clone(), &self.acknowledged.id)?;
        Ok(())
    }
}
