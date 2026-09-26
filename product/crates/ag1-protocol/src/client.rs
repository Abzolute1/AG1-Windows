//! Native client projections, generated with the rest of the wire contract.
use crate::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessageBlock {
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    Tool {
        name: String,
        call_id: String,
        arguments: Value,
    },
    Attachment {
        media_type: String,
        reference: String,
    },
    Unknown {
        value: Value,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ClientMessage {
    pub id: String,
    pub role: String,
    pub blocks: Vec<MessageBlock>,
    /// Execution identity, independent of result arrival order or display text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tool: Option<ToolResultIdentity>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ToolResultIdentity {
    pub call_id: String,
    pub name: String,
    pub failed: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ClientDetails {
    pub authority: String,
    pub messages: Vec<ClientMessage>,
    pub task: Value,
    pub context: Value,
    pub artifacts: Vec<Value>,
    pub memory: Value,
    pub activity: Value,
    pub models: Vec<Value>,
    pub capabilities: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ClientView {
    pub snapshot: BridgeSnapshot,
    pub error: Option<String>,
    pub progress: String,
    pub details: ClientDetails,
}
