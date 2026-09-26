//! Borrow request fields while serializing custody; do not duplicate the entire
//! conversation in a temporary Value. Field order preserves the existing bytes
//! produced by serde_json::Value's sorted object keys (and thus blob addresses).
use ag1_core::runtime::session::Configuration;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_revision: Option<&'a Option<String>>,
    messages: &'a [Value],
    model: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    stats: Option<&'a Value>,
    system_prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<&'a str>,
    tools: &'a [Value],
    version: u8,
}
impl<'a> Envelope<'a> {
    fn new(configuration: &'a Configuration, messages: &'a [Value]) -> Self {
        Self {
            memory_revision: None,
            messages,
            model: &configuration.model,
            stats: None,
            system_prompt: &configuration.system_prompt,
            thinking: None,
            tools: &configuration.tools,
            version: 1,
        }
    }
    fn encode(&self) -> String {
        serde_json::to_string(self).expect("Borrowed JSON values serialize to custody")
    }
}

pub(super) fn input(
    configuration: &Configuration,
    messages: &[Value],
    memory_revision: Option<&Option<String>>,
) -> String {
    let mut envelope = Envelope::new(configuration, messages);
    envelope.memory_revision = memory_revision;
    envelope.encode()
}

pub(super) fn compiled(
    configuration: &Configuration,
    messages: &[Value],
    memory_revision: &Option<String>,
    stats: &Value,
) -> String {
    let mut envelope = Envelope::new(configuration, messages);
    envelope.memory_revision = Some(memory_revision);
    envelope.stats = Some(stats);
    envelope.thinking = Some(&configuration.thinking);
    envelope.encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_existing_custody_bytes_survive_borrowed_serialization() {
        let configuration = Configuration {
            model: json!({"id":"fixture","contextWindow":32768,"compat":{"unknown":true}}),
            thinking: "medium".into(),
            tools: vec![json!({"name":"evidence","parameters":{"type":"object"}})],
            system_prompt: "Keep Unicode β and escaped \"quotes\"\n".into(),
            prompt: None,
            compiler: Default::default(),
        };
        let messages = vec![
            json!({"role":"user","content":"original request"}),
            json!({"role":"toolResult","toolCallId":"x","content":[{"type":"text","text":"line\n\tβ"}],"details":{"null":null,"integer":9007199254740991u64,"array":[false,1.25]}}),
        ];
        // These are the previous native expressions, including key order and
        // absent-vs-null fields. Artifact addresses must remain byte identical.
        let delegated = json!({"version":1,"systemPrompt":configuration.system_prompt,"tools":configuration.tools,"model":configuration.model,"messages":messages}).to_string();
        assert_eq!(input(&configuration, &messages, None), delegated);
        for revision in [None, Some("revision-β".into())] {
            let original = json!({"version":1,"systemPrompt":configuration.system_prompt,"tools":configuration.tools,"model":configuration.model,"messages":messages,"memoryRevision":revision}).to_string();
            assert_eq!(input(&configuration, &messages, Some(&revision)), original);
            let stats = json!({"workingSetReused":true,"tokens":4096,"unknown":null});
            let output = json!({"version":1,"systemPrompt":configuration.system_prompt,"tools":configuration.tools,"model":configuration.model,"thinking":configuration.thinking,"messages":messages,"stats":stats,"memoryRevision":revision}).to_string();
            assert_eq!(
                compiled(&configuration, &messages, &revision, &stats),
                output
            );
        }
        assert_ne!(
            input(&configuration, &messages, None),
            input(&configuration, &messages, Some(&None))
        );
    }
}
