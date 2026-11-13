//! Bounded non-sensitive form continuation. Opaque protocol state never enters
//! UI projections, model context, logs or durable result artifacts.
use super::{
    invalid,
    wire::{Limits, decode_json},
};
use crate::resource_fs::ScanControl;
use ag1_core::{DomainError, tools::mcp::Identity};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_ROUNDS: usize = 3;
pub const MAX_SECONDS: u64 = 120;
#[derive(Clone, Debug)]
pub enum Choices {
    Boolean,
    Enum(Vec<String>),
}
#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub title: String,
    pub description: String,
    pub required: bool,
    pub choices: Choices,
}
#[derive(Clone, Debug)]
pub struct Form {
    pub identity: Identity,
    pub call_id: String,
    pub original_arguments: Value,
    pub message: String,
    pub fields: Vec<Field>,
    pub round: usize,
}
pub enum Answer {
    Accept(BTreeMap<String, Value>),
    Decline,
    Cancel,
}
/// Native UI capability only; peers/renderer JSON cannot construct this port.
pub trait Entry: Send + Sync {
    fn enter(&self, form: &Form, control: &ScanControl<'_>) -> Result<Answer, DomainError>;
}
pub(crate) struct Round {
    key: String,
    state: Option<String>,
    pub form: Form,
}
fn text(value: Option<&Value>, max: usize) -> Result<String, DomainError> {
    value
        .and_then(Value::as_str)
        .filter(|s| s.len() <= max && !s.contains('\0'))
        .map(str::to_owned)
        .ok_or_else(|| invalid("MCP input text"))
}
fn only(value: &serde_json::Map<String, Value>, keys: &[&str]) -> Result<(), DomainError> {
    if value.keys().any(|k| !keys.contains(&k.as_str())) {
        Err(DomainError::Unavailable(
            "MCP input form uses unsupported fields or constraints.".into(),
        ))
    } else {
        Ok(())
    }
}
impl Round {
    pub fn parse(
        envelope: &Value,
        identity: &Identity,
        call_id: &str,
        args: &Value,
        round: usize,
    ) -> Result<Self, DomainError> {
        if round == 0 || round > MAX_ROUNDS {
            return Err(DomainError::LimitExceeded("MCP input rounds".into()));
        }
        let raw = serde_json::to_vec(envelope).map_err(|_| invalid("MCP input envelope"))?;
        decode_json(&raw, Limits::new(64 * 1024, 16, 2048)?)?;
        let result = envelope
            .get("result")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("MCP input result"))?;
        only(
            result,
            &["resultType", "inputRequests", "requestState", "_meta"],
        )?;
        if result.get("resultType") != Some(&json!("input_required")) {
            return Err(invalid("MCP waiting input"));
        }
        let requests=result.get("inputRequests").and_then(Value::as_object).filter(|r|r.len()==1)
            .ok_or_else(||DomainError::Unavailable("MCP supports one user form per input round; state-only retry, sampling and roots requests are unsupported.".into()))?;
        let (key, request) = requests.iter().next().ok_or(DomainError::Conflict)?;
        if !super::definitions::label(key, 128) {
            return Err(invalid("MCP input request key"));
        }
        let request = request
            .as_object()
            .ok_or_else(|| invalid("MCP input request"))?;
        only(request, &["method", "params"])?;
        if request.get("method") != Some(&json!("elicitation/create")) {
            return Err(DomainError::Unavailable(
                "MCP sampling and roots input requests are unsupported.".into(),
            ));
        }
        let params = request
            .get("params")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("MCP form parameters"))?;
        only(params, &["mode", "message", "requestedSchema"])?;
        if params.get("mode").is_some_and(|v| v != "form") {
            return Err(DomainError::Unavailable("MCP URL/private authentication input must use the separate native authentication flow.".into()));
        }
        let message = text(params.get("message"), 4096)?;
        let schema = params
            .get("requestedSchema")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("MCP form schema"))?;
        only(schema, &["type", "properties", "required", "$schema"])?;
        if schema.get("type") != Some(&json!("object")) {
            return Err(invalid("MCP form object schema"));
        }
        if schema.get("$schema").is_some_and(|v| {
            !matches!(
                v.as_str(),
                Some(
                    "https://json-schema.org/draft/2020-12/schema"
                        | "http://json-schema.org/draft-07/schema#"
                )
            )
        }) {
            return Err(invalid("MCP form schema dialect"));
        }
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .filter(|p| !p.is_empty() && p.len() <= 8)
            .ok_or_else(|| invalid("MCP form field count"))?;
        let required: Vec<String> = schema
            .get("required")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|_| invalid("MCP form required fields"))?
            .unwrap_or_default();
        let required: BTreeSet<_> = required.iter().collect();
        if required
            .iter()
            .any(|k| !properties.contains_key(k.as_str()))
        {
            return Err(invalid("MCP form required key"));
        }
        let mut fields = Vec::new();
        for (name, value) in properties {
            if !super::definitions::label(name, 128) {
                return Err(invalid("MCP form field name"));
            }
            let value = value
                .as_object()
                .ok_or_else(|| invalid("MCP form field schema"))?;
            only(value, &["type", "title", "description", "enum"])?;
            let title = value
                .get("title")
                .map(|v| text(Some(v), 256))
                .transpose()?
                .unwrap_or_else(|| name.clone());
            let description = value
                .get("description")
                .map(|v| text(Some(v), 1024))
                .transpose()?
                .unwrap_or_default();
            let choices = match value.get("type").and_then(Value::as_str) {
                Some("boolean") if !value.contains_key("enum") => Choices::Boolean,
                Some("string") => {
                    let values=value.get("enum").and_then(Value::as_array).filter(|a|!a.is_empty()&&a.len()<=16).ok_or_else(||DomainError::Unavailable("MCP input supports booleans and explicit string choices; free text, credentials and numeric forms are unsupported.".into()))?;
                    let values = values
                        .iter()
                        .map(|v| text(Some(v), 256))
                        .collect::<Result<Vec<_>, _>>()?;
                    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
                        return Err(invalid("MCP input duplicate choices"));
                    }
                    Choices::Enum(values)
                }
                _ => {
                    return Err(DomainError::Unavailable(
                        "MCP input supports booleans and explicit string choices only.".into(),
                    ));
                }
            };
            fields.push(Field {
                name: name.clone(),
                title,
                description,
                required: required.contains(name),
                choices,
            });
        }
        let state = result
            .get("requestState")
            .map(|v| text(Some(v), 16 * 1024))
            .transpose()?;
        Ok(Self {
            key: key.clone(),
            state,
            form: Form {
                identity: identity.clone(),
                call_id: call_id.into(),
                original_arguments: args.clone(),
                message,
                fields,
                round,
            },
        })
    }
    pub fn respond(self, answer: Answer) -> Result<Value, DomainError> {
        let response = match answer {
            Answer::Decline => json!({"action":"decline"}),
            Answer::Cancel => return Err(DomainError::Cancelled),
            Answer::Accept(content) => {
                if content
                    .keys()
                    .any(|k| !self.form.fields.iter().any(|f| &f.name == k))
                {
                    return Err(DomainError::PermissionDenied);
                }
                for field in &self.form.fields {
                    match content.get(&field.name) {
                        None if field.required => return Err(DomainError::PermissionDenied),
                        None => {}
                        Some(value) => {
                            let valid = match &field.choices {
                                Choices::Boolean => value.is_boolean(),
                                Choices::Enum(values) => value
                                    .as_str()
                                    .is_some_and(|v| values.iter().any(|s| s == v)),
                            };
                            if !valid {
                                return Err(DomainError::PermissionDenied);
                            }
                        }
                    }
                }
                json!({"action":"accept","content":content})
            }
        };
        let mut fields = json!({"inputResponses":{self.key:response}});
        if let Some(state) = self.state {
            fields["requestState"] = json!(state);
        }
        Ok(fields)
    }
}
