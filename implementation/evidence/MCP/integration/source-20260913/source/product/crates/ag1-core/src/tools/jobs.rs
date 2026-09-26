//! Durable job observations. A persisted PID is descriptive data, never a process capability.
use crate::context::ContextArtifact;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Intent,
    Running,
    Exited,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub operation: String,
    pub call_id: String,
    pub command: String,
    pub owner: String,
    pub epoch: u64,
    pub started_at: u64,
    pub status: Status,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub finished_at: Option<u64>,
    pub result: Option<ContextArtifact>,
    pub notified: bool,
}
impl Record {
    pub fn line(&self, now: u64) -> String {
        let command = super::shell::job_command(&self.command);
        match self.status {
            Status::Running => format!(
                "{} running {}s · {command}",
                self.id,
                (now.saturating_sub(self.started_at) + 500) / 1000
            ),
            Status::Exited => format!(
                "{} exited {} · {command}",
                self.id,
                self.exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into())
            ),
            Status::Intent => format!("{} starting · {command}", self.id),
            Status::Unknown => format!("{} outcome unknown · {command}", self.id),
        }
    }
    pub fn notice_id(&self) -> String {
        format!("job-finished-{}", self.id)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Book {
    pub version: u32,
    pub workspace: String,
    pub session: String,
    pub jobs: Vec<Record>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notice {
    pub id: String,
    pub text: String,
}
