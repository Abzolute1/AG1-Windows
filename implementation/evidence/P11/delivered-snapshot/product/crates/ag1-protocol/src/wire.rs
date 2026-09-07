use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use ts_rs::TS;

pub const MAX_ENVELOPE: usize = 64 * 1024;
pub const MAX_ARTIFACT_PAGE: u32 = 32 * 1024;
pub const MAX_SESSION_QUEUE: usize = 256;
pub const MAX_ENDPOINT_WAITING: usize = 128;
pub const MAX_SUBSCRIBER_BYTES: usize = 1024 * 1024;
pub const SCHEMA_ID: &str = "ag1.protocol.v1";
pub fn schema_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&schemars::schema_for!(WireMessage)).unwrap())
        )
    })
}

macro_rules! wire_id {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
        #[serde(try_from = "String", into = "String")]
        #[ts(type = "string")]
        pub struct $name(String);
        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                if value.is_empty() || value.len() > 128 || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b)) {
                    Err(format!("invalid {}", stringify!($name)))
                } else { Ok(Self(value)) }
            }
        }
        impl From<$name> for String { fn from(value: $name) -> Self { value.0 } }
        impl $name { pub fn as_str(&self) -> &str { &self.0 } }
        impl JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> { stringify!($name).into() }
            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9_.:-]+$"})
            }
        }
    )+};
}
wire_id!(
    WorkspaceId,
    SessionId,
    BranchId,
    OperationId,
    ArtifactId,
    CommandId,
    InstanceId,
    SubscriptionId,
    ApprovalId
);

/// Decimal strings preserve full u64 precision in JavaScript. Canonical spelling only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct Sequence(String);
impl JsonSchema for Sequence {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Sequence".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type": "string", "maxLength": 20, "pattern": "^(0|[1-9][0-9]*)$", "description": "Canonical u64 decimal; host additionally checks <=18446744073709551615."})
    }
}
impl Sequence {
    pub fn new(value: u64) -> Self {
        Self(value.to_string())
    }
    pub fn get(&self) -> u64 {
        self.0.parse().expect("validated decimal")
    }
}
impl From<Sequence> for String {
    fn from(value: Sequence) -> Self {
        value.0
    }
}
impl TryFrom<String> for Sequence {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 20
            || !value.bytes().all(|b| b.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
            || value.parse::<u64>().is_err()
        {
            Err("expected canonical u64 decimal string".into())
        } else {
            Ok(Self(value))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub schema: String,
}
impl Default for Version {
    fn default() -> Self {
        Self {
            major: 1,
            minor: 0,
            schema: schema_hash().into(),
        }
    }
}
impl Version {
    pub fn negotiate(&self) -> Result<Self, ProtocolError> {
        if self.major != 1 || self.schema != schema_hash() {
            return Err(ProtocolError::IncompatibleVersion);
        }
        Ok(Self::default())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProtocolError {
    IncompatibleVersion,
    Malformed { detail: String },
    Oversize { limit: u32, use_artifact: bool },
    QueueFull { queue: String, capacity: u32 },
    Unavailable { family: String },
    StaleContext,
    CommandConflict,
    InvalidApproval,
    NotFound,
    ResyncRequired,
    Capacity { resource: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub workspace: WorkspaceId,
    pub session: SessionId,
    pub instance: InstanceId,
    pub epoch: Sequence,
}

/// Missing is None; explicit JSON null is Some(Value::Null). No loss of custom data.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}
fn required_nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}
fn nullable_operation(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    generator.subschema_for::<Option<OperationView>>()
}
fn nullable_sequence(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    generator.subschema_for::<Option<Sequence>>()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct CommandEnvelope {
    pub version: Version,
    pub id: CommandId,
    pub scope: Scope,
    pub expected_revision: Sequence,
    pub command: Command,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[ts(optional, type = "unknown")]
    pub extension: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PromptInput {
    Inline { text: String },
    Artifact { artifact: ArtifactId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Prompt {
        input: PromptInput,
    },
    Cancel {
        operation: OperationId,
    },
    Approve {
        binding: ApprovalBinding,
        allow: bool,
    },
    CommandResult {
        command: CommandId,
    },
    ArtifactRead {
        artifact: ArtifactId,
        offset: Sequence,
        limit: u32,
    },
    Project {
        action: String,
        args: Value,
    },
    Session {
        action: String,
        args: Value,
    },
    Task {
        action: String,
        args: Value,
    },
    Plan {
        action: String,
        args: Value,
    },
    Files {
        action: String,
        args: Value,
    },
    Artifacts {
        action: String,
        args: Value,
    },
    Memory {
        action: String,
        args: Value,
    },
    Model {
        action: String,
        args: Value,
    },
    Runtime {
        action: String,
        args: Value,
    },
    Diagnostics {
        action: String,
        args: Value,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ApprovalBinding {
    pub id: ApprovalId,
    pub operation: OperationId,
    pub argument_sha256: String,
    pub scope: Scope,
    pub revision: Sequence,
    pub expires_ms: Sequence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Queued,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct OperationView {
    pub operation: OperationId,
    pub status: OperationStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandResult {
    Accepted {
        operation: OperationId,
    },
    Settled {
        operation: OperationView,
    },
    ArtifactPage {
        artifact: ArtifactId,
        offset: Sequence,
        base64: String,
        #[serde(deserialize_with = "required_nullable")]
        #[schemars(required, schema_with = "nullable_sequence")]
        next: Option<Sequence>,
    },
    Rejected {
        error: ProtocolError,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub id: CommandId,
    pub scope: Scope,
    pub watermark: Sequence,
    pub result: CommandResult,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub fake: bool,
    pub scope: Scope,
    pub branch: BranchId,
    pub revision: Sequence,
    pub watermark: Sequence,
    pub queued: Vec<OperationId>,
    #[serde(deserialize_with = "required_nullable")]
    #[schemars(required, schema_with = "nullable_operation")]
    pub last_terminal: Option<OperationView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub scope: Scope,
    pub sequence: Sequence,
    pub revision: Sequence,
    pub operation: OperationView,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub scope: Scope,
    pub after: Sequence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Delivery {
    Events {
        events: Vec<Event>,
        cursor: Cursor,
        more: bool,
    },
    Progress {
        scope: Scope,
        operation: OperationId,
        tick: Sequence,
        text: String,
    },
    Gap {
        snapshot: Snapshot,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Subscription {
    pub id: SubscriptionId,
    pub snapshot: Snapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Handshake {
    pub version: Version,
    pub scope: Scope,
    pub fake: bool,
    pub capabilities: Vec<String>,
}

/// Root includes every boundary type so generated dependencies cannot silently go missing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum WireMessage {
    Handshake(Handshake),
    Command(CommandEnvelope),
    Result(Reply),
    Snapshot(Snapshot),
    Delivery(Delivery),
    Subscription(Subscription),
    Cursor(Cursor),
    Error(ProtocolError),
    Bridge(BridgeFrame),
}

pub fn decode(bytes: &[u8]) -> Result<WireMessage, ProtocolError> {
    if bytes.len() > MAX_ENVELOPE {
        return Err(ProtocolError::Oversize {
            limit: MAX_ENVELOPE as u32,
            use_artifact: true,
        });
    }
    serde_json::from_slice(bytes).map_err(|_| ProtocolError::Malformed {
        detail: "invalid bounded protocol JSON".into(),
    })
}
pub fn encode(value: &WireMessage) -> Result<Vec<u8>, ProtocolError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ProtocolError::Malformed {
        detail: "cannot encode protocol JSON".into(),
    })?;
    if bytes.len() > MAX_ENVELOPE {
        return Err(ProtocolError::Oversize {
            limit: MAX_ENVELOPE as u32,
            use_artifact: true,
        });
    }
    Ok(bytes)
}

impl Scope {
    pub fn from_core(value: &ag1_core::SessionView, instance: InstanceId) -> Self {
        Self {
            workspace: value
                .workspace
                .as_str()
                .to_owned()
                .try_into()
                .expect("validated core ID"),
            session: value
                .session
                .as_str()
                .to_owned()
                .try_into()
                .expect("validated core ID"),
            instance,
            epoch: Sequence::new(value.epoch),
        }
    }
}

/// P04 real SDK projections. Full payloads stay in TS-owned evidence; views are bounded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct BridgeMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    pub evidence: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct BridgeQuestion {
    pub binding: ApprovalBinding,
    pub title: String,
    pub kind: String,
    pub options: Vec<String>,
    pub evidence: String,
    pub truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct BridgeSnapshot {
    pub version: Version,
    pub scope: Scope,
    pub revision: Sequence,
    pub watermark: Sequence,
    pub workspace_path: String,
    pub status: String,
    pub active_operation: Option<OperationId>,
    pub messages: Vec<BridgeMessage>,
    pub message_count: u32,
    pub pending: Vec<BridgeQuestion>,
    pub model: String,
    pub diagnostic: Option<String>,
    pub session_file: Option<String>,
    pub fixture_provider: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BridgeFrame {
    Snapshot {
        snapshot: BridgeSnapshot,
    },
    Reply {
        id: CommandId,
        scope: Scope,
        revision: Sequence,
        durable: bool,
        status: String,
        data: Value,
    },
    Progress {
        scope: Scope,
        operation: OperationId,
        text: String,
    },
    Fatal {
        detail: String,
    },
}
