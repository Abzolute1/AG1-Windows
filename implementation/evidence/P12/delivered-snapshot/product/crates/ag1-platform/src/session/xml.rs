//! XML candidates enter the ordinary durable native assistant/tool lifecycle.
//! The immutable provider output artifact remains the unmodified source.
use ag1_core::{DomainError, runtime::xml};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct Fallback {
    iterations: usize,
    seen: BTreeSet<String>,
}
pub(super) struct Prepared {
    pub guidance: Option<String>,
    pub event: Value,
}
impl Fallback {
    pub fn prepare(
        &mut self,
        message: &mut Value,
        tools: &[String],
        operation: &str,
    ) -> Result<Option<Prepared>, DomainError> {
        if self.iterations >= 3
            || message["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "toolCall"))
        {
            return Ok(None);
        }
        let text = if let Some(text) = message["content"].as_str() {
            text.into()
        } else {
            message["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|block| match block["type"].as_str() {
                    Some("text") => Some(block["text"].as_str().unwrap_or("")),
                    Some("thinking") => Some(block["thinking"].as_str().unwrap_or("")),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        if !text.contains('<') {
            return Ok(None);
        }
        let identity =
            ag1_storage::digest(json!([message["timestamp"], text]).to_string().as_bytes());
        if self.seen.contains(&identity) {
            return Ok(None);
        }
        let calls = xml::parse(&text, tools)?;
        if calls.is_empty() {
            return Ok(None);
        }
        let analysis = xml::analyze(
            calls.len(),
            text.encode_utf16().count(),
            message["stopReason"].as_str(),
            self.iterations,
        );
        let names: Vec<_> = calls.iter().map(|call| call.name.clone()).collect();
        let guidance = xml::guidance(&analysis, &names);
        let mut content = if let Some(text) = message["content"].as_str() {
            vec![json!({"type":"text","text":text})]
        } else {
            message["content"].as_array().cloned().unwrap_or_default()
        };
        let prefix = ag1_storage::digest(operation.as_bytes());
        let mut identities = vec![];
        for (index, call) in calls.into_iter().enumerate() {
            let id = format!("xml-{}-{index}", &prefix[..32]);
            identities.push(json!({"id":id,"sourceId":call.id,"name":call.name}));
            content.push(
                json!({"type":"toolCall","id":id,"name":call.name,"arguments":call.arguments}),
            );
        }
        message["content"] = json!(content);
        message["nativeXmlFallback"] = json!({"version":1,"sourceArtifact":message["nativeOutputArtifact"],"analysis":analysis,"calls":identities});
        self.iterations += 1;
        self.seen.insert(identity);
        Ok(Some(Prepared {
            guidance,
            event: json!({"type":"xml_tool_call","iteration":self.iterations,"analysis":analysis,"calls":identities}),
        }))
    }
}
