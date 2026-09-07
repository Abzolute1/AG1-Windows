//! One retry/compaction continuation owner, using an injected clock. Native
//! request stamps fence full session authority, not JavaScript object identity.
use super::boundary::Stamp;
use crate::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    Retry,
    Compaction,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Run(Owner),
    StateChanged(Owner),
}
struct Scheduled {
    owner: Owner,
    stamp: Stamp,
    due: u64,
}
#[derive(Default)]
pub struct Recovery {
    owner: Option<Owner>,
    scheduled: Option<Scheduled>,
}
impl Recovery {
    pub fn owner(&self) -> Option<Owner> {
        self.owner
    }
    pub fn is_scheduled(&self) -> bool {
        self.scheduled.is_some()
    }
    pub fn try_arm(&mut self, owner: Owner) -> bool {
        if self.owner.is_some_and(|current| current != owner) {
            return false;
        }
        self.owner = Some(owner);
        true
    }
    pub fn clear(&mut self, owner: Option<Owner>) {
        if owner.is_none() || self.owner == owner {
            self.owner = None;
            self.scheduled = None;
        }
    }
    /// Same-owner scheduling replaces the earlier timer; a competing owner keeps
    /// its claim. The embedding emits already_scheduled on a false result.
    pub fn schedule(
        &mut self,
        owner: Owner,
        stamp: &Stamp,
        now: u64,
        delay: u64,
    ) -> Result<bool, DomainError> {
        if delay > 86_400_000 {
            return Err(DomainError::LimitExceeded("recovery delay".into()));
        }
        let due = now.checked_add(delay).ok_or(DomainError::Conflict)?;
        if !self.try_arm(owner) {
            return Ok(false);
        }
        self.scheduled = Some(Scheduled {
            owner,
            stamp: stamp.clone(),
            due,
        });
        Ok(true)
    }
    /// Clear ownership before returning Run, so a failed continuation can arm a
    /// new recovery without retaining the completed callback's claim. The session
    /// must still pass Boundary::prepare/admit before calling a provider.
    pub fn poll(&mut self, now: u64, current: &Stamp, is_streaming: bool) -> Option<Decision> {
        if self.scheduled.as_ref().is_none_or(|r| now < r.due) {
            return None;
        }
        let replay = self.scheduled.take()?;
        self.owner = None;
        Some(if is_streaming || replay.stamp != *current {
            Decision::StateChanged(replay.owner)
        } else {
            Decision::Run(replay.owner)
        })
    }
}
