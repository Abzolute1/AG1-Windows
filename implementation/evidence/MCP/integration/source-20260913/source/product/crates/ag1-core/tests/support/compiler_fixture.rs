use ag1_core::{
    DomainError,
    context::{ContextArtifact, ContextArtifactPort, compiler::CompileOptions},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
pub struct FixtureStore {
    pub records: Vec<Value>,
    pub texts: BTreeMap<String, String>,
    pub leases: BTreeSet<String>,
    pub fail: bool,
}
impl ContextArtifactPort for FixtureStore {
    fn put_evidence(
        &mut self,
        text: &str,
        kind: &str,
        producer: &Value,
        salient: &[String],
        parent: Option<&str>,
    ) -> Result<ContextArtifact, DomainError> {
        let record = self.put_text(
            text,
            kind,
            producer["name"].as_str().unwrap_or(""),
            producer["callId"].as_str().unwrap_or(""),
            salient,
        )?;
        let row = self.records.last_mut().unwrap();
        row["producer"] = producer.clone();
        if let Some(parent) = parent {
            row["parentEntryId"] = parent.into();
        }
        Ok(record)
    }

    fn put_text(
        &mut self,
        text: &str,
        kind: &str,
        tool: &str,
        call_id: &str,
        salient: &[String],
    ) -> Result<ContextArtifact, DomainError> {
        if self.fail {
            return Err(DomainError::DurabilityUnknown);
        }
        let id = format!("art_{:032x}", self.records.len() + 1);
        let lines = if text.is_empty() {
            0
        } else {
            text.bytes().filter(|b| *b == b'\n').count() + 1 - usize::from(text.ends_with('\n'))
        };
        let record = ContextArtifact {
            id: id.clone(),
            kind: kind.into(),
            bytes: text.len() as u64,
            sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
            line_count: Some(lines as u64),
            salient_lines: salient.to_vec(),
        };
        let mut value = serde_json::to_value(&record).unwrap();
        value["producer"] = json!({"type":"tool","name":tool,"callId":call_id});
        self.records.push(value);
        self.texts.insert(id, text.into());
        Ok(record)
    }
    fn get(&self, id: &str) -> Result<ContextArtifact, DomainError> {
        let record = self
            .records
            .iter()
            .find(|r| r["id"] == id)
            .ok_or(DomainError::NotFound)?;
        serde_json::from_value(record.clone())
            .map_err(|_| DomainError::InvalidInput("fixture record".into()))
    }
    fn leased_ids(&self) -> Result<Vec<String>, DomainError> {
        Ok(self.leases.iter().cloned().collect())
    }
}
pub fn recipe_input(recipe: &Value, store: &mut FixtureStore) -> (Vec<Value>, CompileOptions) {
    let user = |text: &str| json!({"role":"user","content":text,"timestamp":1});
    let mut messages = vec![user("Implement the repository task")];
    let rounds = recipe["rounds"].as_u64().unwrap();
    let seed = recipe["seed"].as_str().map(|s| {
        let record = store
            .put_text(
                &format!(
                    "CURRENT_CONTRACT: preserve exact evidence\n{}",
                    "seed ".repeat(1000)
                ),
                "test-output",
                "check",
                "seed",
                &["CURRENT_CONTRACT: preserve exact evidence".into()],
            )
            .unwrap();
        if s == "lease" {
            store.leases.insert(record.id.clone());
        }
        record
    });
    for i in 0..rounds {
        let tool = recipe["tool"].as_str().unwrap_or("bash");
        let thinking = if i >= rounds - 3 {
            recipe["lastThinkingBytes"].as_u64()
        } else {
            None
        }
        .unwrap_or(recipe["thinkingBytes"].as_u64().unwrap());
        let args = if let Some(bytes) = recipe["payloadBytes"].as_u64() {
            json!({"content":"x".repeat(bytes as usize),"path":"src/a.rs"})
        } else if tool == "read" {
            json!({"path":"src/a.rs"})
        } else {
            json!({"command":"check"})
        };
        messages.push(json!({"role":"assistant","content":[{"type":"thinking","thinking":"r".repeat(thinking as usize)},{"type":"toolCall","id":format!("c{i}"),"name":tool,"arguments":args}],"api":"openai-completions","provider":"local","model":"fixture","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0},"stopReason":"toolUse","timestamp":1}));
        let raw = format!(
            "round {i}\nCURRENT_CONTRACT: keep evidence {i}\n{}\nno actionable evidence follows",
            if recipe["unicode"] == true {
                "β🌙"
            } else {
                "x"
            }
            .repeat(recipe["textBytes"].as_u64().unwrap() as usize)
        );
        let content: Vec<_> = (0..recipe["blocks"].as_u64().unwrap_or(1))
            .map(|_| json!({"type":"text","text":raw}))
            .collect();
        let mut result = json!({"role":"toolResult","toolCallId":format!("c{i}"),"toolName":tool,"content":content,"isError":false,"timestamp":1,"details":{"fixture":true}});
        if i == 0
            && let Some(seed) = &seed
        {
            if recipe["seed"] == "read" {
                result["toolName"] = json!("artifact");
                result["details"] = json!({"action":"read","id":seed.id});
            } else {
                result["content"] = json!([{"type":"text","text":format!("artifact: {}\nfull_result: exact artifact, paged by UTF-8 byte offset",seed.id)}]);
                result["details"] = json!({"id":seed.id});
            }
        }
        messages.push(result);
    }
    if recipe["finalUser"] == true {
        messages.push(user("Continue using the exact evidence above"));
    }
    let options = CompileOptions {
        context_window: recipe["window"].as_u64().unwrap(),
        recent_assistant_turns: recipe["recent"].as_u64().map(|n| n as usize),
        max_resident_artifact_metadata_tokens: recipe["metadataBudget"].as_u64(),
        fixed_tokens: recipe["fixed"].as_u64(),
        ..Default::default()
    };
    (messages, options)
}
