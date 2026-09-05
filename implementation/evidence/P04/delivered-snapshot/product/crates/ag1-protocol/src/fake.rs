//! In-memory, single-owner protocol fixture host. Never executes a model, process or file operation.
//! Accepted commands/results survive reconnects to this instance only. P05 supplies durability.
use crate::*;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};

pub const MAX_COMMAND_RECORDS: usize = 4096;
pub const MAX_REPLAY_EVENTS: usize = 1024;
pub const MAX_SUBSCRIBERS: usize = 64;
pub const MAX_FIXTURE_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
struct Admission {
    fingerprint: [u8; 32],
    result: CommandResult,
}
struct Subscriber {
    cursor: Cursor,
    progress: VecDeque<(Delivery, usize)>,
    bytes: usize,
    gap: bool,
}

/// Clock advances only when a test calls advance_ms; work advances only through step().
pub struct FakeHost {
    scope: Scope,
    revision: u64,
    sequence: u64,
    now_ms: u64,
    serial: u64,
    queue: VecDeque<OperationId>,
    admissions: BTreeMap<CommandId, Admission>,
    operations: BTreeMap<OperationId, OperationStatus>,
    events: VecDeque<Event>,
    subscribers: BTreeMap<SubscriptionId, Subscriber>,
    approvals: BTreeMap<ApprovalId, ApprovalBinding>,
    artifacts: BTreeMap<ArtifactId, Vec<u8>>,
    artifact_bytes: usize,
    last_terminal: Option<OperationView>,
    pub peak_subscriber_bytes: usize,
}

fn id<T: TryFrom<String>>(value: impl Into<String>) -> T
where
    T::Error: std::fmt::Debug,
{
    value.into().try_into().unwrap()
}
fn rejected(error: ProtocolError) -> CommandResult {
    CommandResult::Rejected { error }
}

impl FakeHost {
    pub fn new(instance: InstanceId) -> Self {
        Self {
            scope: Scope {
                workspace: id("fixture-workspace"),
                session: id("fixture-session"),
                instance,
                epoch: Sequence::new(1),
            },
            revision: 0,
            sequence: 0,
            now_ms: 0,
            serial: 0,
            queue: VecDeque::new(),
            admissions: BTreeMap::new(),
            operations: BTreeMap::new(),
            events: VecDeque::new(),
            subscribers: BTreeMap::new(),
            approvals: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            artifact_bytes: 0,
            last_terminal: None,
            peak_subscriber_bytes: 0,
        }
    }
    pub fn handshake(&self, version: &Version) -> Result<Handshake, ProtocolError> {
        Ok(Handshake {
            version: version.negotiate()?,
            scope: self.scope.clone(),
            fake: true,
            capabilities: [
                "fixture_prompt",
                "cancel",
                "approve",
                "command_result",
                "artifact_read",
                "snapshot_subscribe",
            ]
            .map(String::from)
            .to_vec(),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            fake: true,
            scope: self.scope.clone(),
            branch: id("fixture-branch"),
            revision: Sequence::new(self.revision),
            watermark: Sequence::new(self.sequence),
            queued: self.queue.iter().cloned().collect(),
            last_terminal: self.last_terminal.clone(),
        }
    }
    pub fn envelope(&self, command_id: &str, command: Command) -> CommandEnvelope {
        CommandEnvelope {
            version: Version::default(),
            id: id(command_id),
            scope: self.scope.clone(),
            expected_revision: Sequence::new(self.revision),
            command,
            extension: None,
        }
    }
    pub fn advance_ms(&mut self, elapsed: u64) {
        self.now_ms = self.now_ms.saturating_add(elapsed);
    }
    pub fn dispatch(&mut self, bytes: &[u8]) -> Result<Vec<u8>, ProtocolError> {
        let WireMessage::Command(envelope) = decode(bytes)? else {
            return Err(ProtocolError::Malformed {
                detail: "expected command".into(),
            });
        };
        encode(&WireMessage::Result(self.command(envelope)))
    }
    pub fn command(&mut self, envelope: CommandEnvelope) -> Reply {
        let result = self.admit(&envelope);
        Reply {
            id: envelope.id,
            scope: self.scope.clone(),
            watermark: Sequence::new(self.sequence),
            result,
        }
    }
    fn admit(&mut self, envelope: &CommandEnvelope) -> CommandResult {
        if let Err(error) = encode(&WireMessage::Command(envelope.clone())) {
            return rejected(error);
        }
        if let Err(error) = envelope.version.negotiate() {
            return rejected(error);
        }
        if envelope.scope != self.scope {
            return rejected(ProtocolError::StaleContext);
        }
        let fingerprint: [u8; 32] = Sha256::digest(serde_json::to_vec(envelope).unwrap()).into();
        // Lookup precedes revision validation: a lost response may be retried after later commits.
        if let Some(old) = self.admissions.get(&envelope.id) {
            return if old.fingerprint == fingerprint {
                old.result.clone()
            } else {
                rejected(ProtocolError::CommandConflict)
            };
        }
        if envelope.expected_revision.get() != self.revision {
            return rejected(ProtocolError::StaleContext);
        }
        // Reads do not consume retained admission slots or mutate revision.
        match &envelope.command {
            Command::CommandResult { command } => return self.lookup(command),
            Command::ArtifactRead {
                artifact,
                offset,
                limit,
            } => return self.read_artifact(artifact, offset, *limit),
            _ => {}
        }
        if self.admissions.len() >= MAX_COMMAND_RECORDS {
            return rejected(ProtocolError::Capacity {
                resource: "fixture command results; new instance required".into(),
            });
        }
        let result = match &envelope.command {
            Command::Prompt { input } => {
                // Retain room for approval/cancellation of every queued operation.
                if self.admissions.len() >= MAX_COMMAND_RECORDS - 2 * MAX_SESSION_QUEUE {
                    return rejected(ProtocolError::Capacity {
                        resource: "fixture command admission history".into(),
                    });
                }
                if self.queue.len() >= MAX_SESSION_QUEUE {
                    return rejected(ProtocolError::QueueFull {
                        queue: "session".into(),
                        capacity: MAX_SESSION_QUEUE as u32,
                    });
                }
                if let PromptInput::Artifact { artifact } = input
                    && !self.artifacts.contains_key(artifact)
                {
                    return rejected(ProtocolError::NotFound);
                }
                self.serial += 1;
                let operation: OperationId = id(format!("fixture-op-{}", self.serial));
                self.queue.push_back(operation.clone());
                self.transition(operation.clone(), OperationStatus::Queued);
                CommandResult::Accepted { operation }
            }
            Command::Cancel { operation } => {
                let Some(status) = self.operations.get(operation).cloned() else {
                    return rejected(ProtocolError::NotFound);
                };
                if status == OperationStatus::Queued {
                    self.queue.retain(|op| op != operation);
                    self.transition(operation.clone(), OperationStatus::Cancelled);
                }
                CommandResult::Settled {
                    operation: OperationView {
                        operation: operation.clone(),
                        status: self.operations[operation].clone(),
                    },
                }
            }
            Command::Approve { binding, allow } => {
                if self.approvals.get(&binding.id) != Some(binding)
                    || binding.scope != self.scope
                    || binding.revision.get() != self.revision
                    || binding.expires_ms.get() <= self.now_ms
                    || self.operations.get(&binding.operation) != Some(&OperationStatus::Queued)
                {
                    return rejected(ProtocolError::InvalidApproval);
                }
                self.approvals.remove(&binding.id);
                if !allow {
                    self.queue.retain(|op| op != &binding.operation);
                    self.transition(binding.operation.clone(), OperationStatus::Cancelled);
                }
                CommandResult::Settled {
                    operation: OperationView {
                        operation: binding.operation.clone(),
                        status: self.operations[&binding.operation].clone(),
                    },
                }
            }
            Command::Project { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "project".into(),
                });
            }
            Command::Session { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "session".into(),
                });
            }
            Command::Task { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "task".into(),
                });
            }
            Command::Plan { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "plan".into(),
                });
            }
            Command::Files { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "files".into(),
                });
            }
            Command::Artifacts { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "artifacts".into(),
                });
            }
            Command::Memory { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "memory".into(),
                });
            }
            Command::Model { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "model".into(),
                });
            }
            Command::Runtime { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "runtime".into(),
                });
            }
            Command::Diagnostics { .. } => {
                return rejected(ProtocolError::Unavailable {
                    family: "diagnostics".into(),
                });
            }
            Command::CommandResult { .. } | Command::ArtifactRead { .. } => unreachable!(),
        };
        self.admissions.insert(
            envelope.id.clone(),
            Admission {
                fingerprint,
                result: result.clone(),
            },
        );
        result
    }
    pub fn lookup(&self, command: &CommandId) -> CommandResult {
        match self.admissions.get(command).map(|a| &a.result) {
            Some(CommandResult::Accepted { operation }) => CommandResult::Settled {
                operation: OperationView {
                    operation: operation.clone(),
                    status: self.operations[operation].clone(),
                },
            },
            Some(result) => result.clone(),
            None => rejected(ProtocolError::NotFound),
        }
    }
    fn transition(&mut self, operation: OperationId, status: OperationStatus) {
        self.revision += 1;
        self.sequence += 1;
        self.operations.insert(operation.clone(), status.clone());
        let view = OperationView { operation, status };
        if view.status != OperationStatus::Queued {
            self.approvals
                .retain(|_, binding| binding.operation != view.operation);
            self.last_terminal = Some(view.clone());
        }
        self.events.push_back(Event {
            scope: self.scope.clone(),
            sequence: Sequence::new(self.sequence),
            revision: Sequence::new(self.revision),
            operation: view,
        });
        if self.events.len() > MAX_REPLAY_EVENTS {
            self.events.pop_front();
        }
    }
    /// Completes one fixture operation; approval-blocked work cannot execute.
    pub fn step(&mut self) -> bool {
        let Some(operation) = self.queue.front().cloned() else {
            return false;
        };
        if self
            .approvals
            .values()
            .any(|binding| binding.operation == operation)
        {
            return false;
        }
        self.queue.pop_front();
        self.transition(operation, OperationStatus::Completed);
        true
    }
    /// Fixture setup only. The eventual host must derive this hash from admitted, immutable arguments.
    pub fn require_approval(
        &mut self,
        operation: OperationId,
        argument_sha256: String,
        expires_ms: u64,
    ) -> Result<ApprovalBinding, ProtocolError> {
        if self.operations.get(&operation) != Some(&OperationStatus::Queued)
            || self.approvals.values().any(|b| b.operation == operation)
            || argument_sha256.len() != 64
            || !argument_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || expires_ms <= self.now_ms
        {
            return Err(ProtocolError::InvalidApproval);
        }
        self.serial += 1;
        let binding = ApprovalBinding {
            id: id(format!("fixture-approval-{}", self.serial)),
            operation,
            argument_sha256,
            scope: self.scope.clone(),
            revision: Sequence::new(self.revision),
            expires_ms: Sequence::new(expires_ms),
        };
        self.approvals.insert(binding.id.clone(), binding.clone());
        Ok(binding)
    }
    pub fn install_artifact(
        &mut self,
        artifact: ArtifactId,
        bytes: Vec<u8>,
    ) -> Result<(), ProtocolError> {
        if self.artifacts.contains_key(&artifact) {
            return Err(ProtocolError::CommandConflict);
        }
        if self.artifact_bytes + bytes.len() > MAX_FIXTURE_ARTIFACT_BYTES
            || self.artifacts.len() >= 256
        {
            return Err(ProtocolError::Capacity {
                resource: "fixture artifacts".into(),
            });
        }
        self.artifact_bytes += bytes.len();
        self.artifacts.insert(artifact, bytes);
        Ok(())
    }
    fn read_artifact(&self, artifact: &ArtifactId, offset: &Sequence, limit: u32) -> CommandResult {
        if limit == 0 || limit > MAX_ARTIFACT_PAGE {
            return rejected(ProtocolError::Oversize {
                limit: MAX_ARTIFACT_PAGE,
                use_artifact: false,
            });
        }
        let Some(bytes) = self.artifacts.get(artifact) else {
            return rejected(ProtocolError::NotFound);
        };
        let Ok(start) = usize::try_from(offset.get()) else {
            return rejected(ProtocolError::NotFound);
        };
        if start > bytes.len() {
            return rejected(ProtocolError::NotFound);
        }
        let end = start.saturating_add(limit as usize).min(bytes.len());
        CommandResult::ArtifactPage {
            artifact: artifact.clone(),
            offset: offset.clone(),
            base64: base64::engine::general_purpose::STANDARD.encode(&bytes[start..end]),
            next: (end < bytes.len()).then(|| Sequence::new(end as u64)),
        }
    }
    /// Taking &mut self is the single-owner barrier: snapshot and listener registration cannot interleave.
    pub fn subscribe(&mut self) -> Result<Subscription, ProtocolError> {
        if self.subscribers.len() >= MAX_SUBSCRIBERS {
            return Err(ProtocolError::Capacity {
                resource: "subscribers".into(),
            });
        }
        self.serial += 1;
        let subscription: SubscriptionId = id(format!("fixture-sub-{}", self.serial));
        let snapshot = self.snapshot();
        self.subscribers.insert(
            subscription.clone(),
            Subscriber {
                cursor: Cursor {
                    scope: self.scope.clone(),
                    after: snapshot.watermark.clone(),
                },
                progress: VecDeque::new(),
                bytes: 0,
                gap: false,
            },
        );
        Ok(Subscription {
            id: subscription,
            snapshot,
        })
    }
    pub fn unsubscribe(&mut self, id: &SubscriptionId) {
        self.subscribers.remove(id);
    }
    pub fn replay(&self, cursor: &Cursor) -> Delivery {
        let after = cursor.after.get();
        if cursor.scope != self.scope
            || after > self.sequence
            || self
                .events
                .front()
                .is_some_and(|event| after < event.sequence.get().saturating_sub(1))
        {
            return Delivery::Gap {
                snapshot: self.snapshot(),
            };
        }
        // 32 events with bounded IDs are well below the 64 KiB envelope ceiling.
        let events: Vec<_> = self
            .events
            .iter()
            .filter(|event| event.sequence.get() > after)
            .take(32)
            .cloned()
            .collect();
        let through = events
            .last()
            .map(|e| e.sequence.clone())
            .unwrap_or_else(|| cursor.after.clone());
        Delivery::Events {
            events,
            more: through.get() < self.sequence,
            cursor: Cursor {
                scope: self.scope.clone(),
                after: through,
            },
        }
    }
    pub fn progress(&mut self, operation: &OperationId, text: String) -> Result<(), ProtocolError> {
        if self.operations.get(operation) != Some(&OperationStatus::Queued) {
            return Err(ProtocolError::NotFound);
        }
        let delivery = Delivery::Progress {
            scope: self.scope.clone(),
            operation: operation.clone(),
            tick: Sequence::new(self.now_ms),
            text,
        };
        let size = encode(&WireMessage::Delivery(delivery.clone()))?.len();
        for subscriber in self.subscribers.values_mut() {
            if subscriber.bytes + size > MAX_SUBSCRIBER_BYTES {
                subscriber.progress.clear();
                subscriber.bytes = 0;
                subscriber.gap = true;
            }
            subscriber.progress.push_back((delivery.clone(), size));
            subscriber.bytes += size;
            self.peak_subscriber_bytes = self.peak_subscriber_bytes.max(subscriber.bytes);
        }
        Ok(())
    }
    pub fn queued_bytes(&self, id: &SubscriptionId) -> Option<usize> {
        self.subscribers.get(id).map(|s| s.bytes)
    }
    pub fn poll(&mut self, id: &SubscriptionId) -> Result<Delivery, ProtocolError> {
        let subscriber = self.subscribers.get(id).ok_or(ProtocolError::NotFound)?;
        let replay = if subscriber.gap {
            Delivery::Gap {
                snapshot: self.snapshot(),
            }
        } else {
            self.replay(&subscriber.cursor)
        };
        let subscriber = self.subscribers.get_mut(id).unwrap();
        match &replay {
            Delivery::Gap { snapshot } => {
                subscriber.cursor = Cursor {
                    scope: snapshot.scope.clone(),
                    after: snapshot.watermark.clone(),
                };
                subscriber.gap = false;
                subscriber.bytes = 0;
                subscriber.progress.clear();
                Ok(replay)
            }
            Delivery::Events { events, cursor, .. } if !events.is_empty() => {
                subscriber.cursor = cursor.clone();
                Ok(replay)
            }
            _ => {
                while let Some((progress, size)) = subscriber.progress.pop_front() {
                    subscriber.bytes -= size;
                    // Do not display stale progress after recovering terminal state.
                    if let Delivery::Progress { operation, .. } = &progress
                        && self.operations.get(operation) == Some(&OperationStatus::Queued)
                    {
                        return Ok(progress);
                    }
                }
                Ok(replay)
            }
        }
    }
    /// Simulates host death. Caller supplies a fresh instance; no work is automatically replayed.
    pub fn restart(&mut self, instance: InstanceId) -> Result<(), ProtocolError> {
        if instance == self.scope.instance {
            return Err(ProtocolError::StaleContext);
        }
        let epoch = self
            .scope
            .epoch
            .get()
            .checked_add(1)
            .ok_or(ProtocolError::StaleContext)?;
        *self = Self::new(instance);
        self.scope.epoch = Sequence::new(epoch);
        Ok(())
    }
}

/// Minimal client projection used to prove late/duplicate/out-of-order event behavior.
pub struct Projection {
    pub snapshot: Snapshot,
    pub needs_resync: bool,
}
impl Projection {
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            snapshot,
            needs_resync: false,
        }
    }
    pub fn apply(&mut self, event: &Event) -> Result<bool, ProtocolError> {
        if event.scope != self.snapshot.scope {
            return Err(ProtocolError::StaleContext);
        }
        if event.sequence.get() <= self.snapshot.watermark.get() {
            return Ok(false);
        }
        if self.needs_resync
            || Some(event.sequence.get()) != self.snapshot.watermark.get().checked_add(1)
            || Some(event.revision.get()) != self.snapshot.revision.get().checked_add(1)
        {
            self.needs_resync = true;
            return Err(ProtocolError::ResyncRequired);
        }
        self.snapshot.watermark = event.sequence.clone();
        self.snapshot.revision = event.revision.clone();
        if event.operation.status == OperationStatus::Queued {
            self.snapshot.queued.push(event.operation.operation.clone());
        } else {
            self.snapshot
                .queued
                .retain(|op| op != &event.operation.operation);
            self.snapshot.last_terminal = Some(event.operation.clone());
        }
        Ok(true)
    }
}
