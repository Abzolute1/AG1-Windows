//! Native-selected resource/prompt definitions in the ordinary MCP tool origin.
//! These schemas bind a specific catalog entry; arguments cannot select methods,
//! servers, resource URIs, prompt names or protocol continuation state.
use super::{Binding, Identity, SelectedDefinition, hash, invalid, label};
use crate::{
    DomainError,
    tools::schema::{self, Schema},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    #[default]
    Tool,
    Resource,
    ResourceTemplate,
    Prompt,
}
impl Kind {
    pub fn is_tool(&self) -> bool {
        *self == Self::Tool
    }
    pub fn key_field(self) -> &'static str {
        match self {
            Self::Resource => "uri",
            Self::ResourceTemplate => "uriTemplate",
            _ => "name",
        }
    }
    pub fn method(self) -> &'static str {
        match self {
            Self::Tool => "tools/call",
            Self::Resource | Self::ResourceTemplate => "resources/read",
            Self::Prompt => "prompts/get",
        }
    }
}
fn uri(value: &str) -> Result<(), DomainError> {
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || url::Url::parse(value).is_err()
    {
        return Err(invalid());
    }
    Ok(())
}
/// Deliberately limited RFC 6570 subset: single simple string variables, all
/// explicit, max 32 distinct names. No operators, modifiers, lists or maps.
fn variables(template: &str) -> Result<BTreeSet<String>, DomainError> {
    if template.len() > 4096 || template.chars().any(char::is_control) {
        return Err(invalid());
    }
    let mut names = BTreeSet::new();
    let mut rest = template;
    let mut probe = String::new();
    while let Some((literal, tail)) = rest.split_once('{') {
        if literal.contains('}') {
            return Err(invalid());
        }
        let (name, tail) = tail.split_once('}').ok_or_else(invalid)?;
        if name.is_empty()
            || name.len() > 256
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(DomainError::Unavailable(
                "MCP URI template expression unsupported".into(),
            ));
        }
        names.insert(name.to_owned());
        if names.len() > 32 {
            return Err(DomainError::LimitExceeded("MCP template variables".into()));
        }
        probe.push_str(literal);
        probe.push('x');
        rest = tail;
    }
    if rest.contains('}') || names.is_empty() {
        return Err(invalid());
    }
    probe.push_str(rest);
    uri(&probe)?;
    Ok(names)
}
impl SelectedDefinition {
    pub fn capability(binding: Binding, kind: Kind, original: &Value) -> Result<Self, DomainError> {
        if kind == Kind::Tool {
            return Self::new(binding, original);
        }
        binding.validate()?;
        schema::bounded(original, 1024 * 1024, 32)?;
        if !original["name"].as_str().is_some_and(label)
            || original
                .get("description")
                .is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= 16 * 1024))
        {
            return Err(invalid());
        }
        let key = original[kind.key_field()].as_str().ok_or_else(invalid)?;
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        match kind {
            Kind::Resource => uri(key)?,
            Kind::ResourceTemplate => {
                for name in variables(key)? {
                    properties.insert(name.clone(), json!({"type":"string","maxLength":4096}));
                    required.push(name);
                }
            }
            Kind::Prompt => {
                if !label(key) {
                    return Err(invalid());
                }
                if let Some(args) = original.get("arguments") {
                    let args = args
                        .as_array()
                        .filter(|a| a.len() <= 64)
                        .ok_or_else(invalid)?;
                    for arg in args {
                        let name = arg["name"]
                            .as_str()
                            .filter(|s| label(s))
                            .ok_or_else(invalid)?;
                        if arg.get("required").is_some_and(|v| !v.is_boolean())
                            || arg
                                .get("description")
                                .is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= 16 * 1024))
                            || properties.contains_key(name)
                        {
                            return Err(invalid());
                        }
                        properties.insert(name.into(), json!({"type":"string","maxLength":4096}));
                        if arg["required"] == true {
                            required.push(name.into());
                        }
                    }
                }
            }
            Kind::Tool => unreachable!(),
        }
        let identity = Identity {
            execution_scope: None,
            binding,
            kind,
            original_name: key.into(),
            metadata_digest: hash(&json!([kind, original])),
        };
        identity.validate()?;
        let parameters = json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
        Schema::parse(&parameters)?;
        let metadata = json!({"name":identity.wire_name(),"description":format!("Explicit MCP {} for {}. {}",kind.method(),key,original["description"].as_str().unwrap_or_default()),"parameters":parameters,
            "sourceInfo":{"source":"mcp","serverId":identity.binding.server_id,"originalName":identity.original_name,"kind":kind,"scope":identity.binding.scope,"identityDigest":identity.target_revision()}});
        Ok(Self {
            identity,
            metadata,
            reviewed_policy: None,
        })
    }
    pub fn protocol_params(&self, args: &Value) -> Result<Value, DomainError> {
        schema::bounded(args, 256 * 1024, 32)?;
        Schema::parse(&self.metadata["parameters"])?.validate(args)?;
        self.identity.protocol_params(args)
    }
}
impl Identity {
    /// Native entry identity determines routing. No caller-controlled method/URI.
    pub fn protocol_params(&self, args: &Value) -> Result<Value, DomainError> {
        self.validate()?;
        Ok(match self.kind {
            Kind::Tool | Kind::Prompt => json!({"name":self.original_name,"arguments":args}),
            Kind::Resource => {
                uri(&self.original_name)?;
                json!({"uri":self.original_name})
            }
            Kind::ResourceTemplate => {
                let names = variables(&self.original_name)?;
                let args = args.as_object().ok_or_else(invalid)?;
                if args.len() != names.len() || args.keys().any(|n| !names.contains(n)) {
                    return Err(invalid());
                }
                let mut expanded = String::new();
                let mut rest = self.original_name.as_str();
                while let Some((literal, tail)) = rest.split_once('{') {
                    let (name, tail) = tail.split_once('}').ok_or_else(invalid)?;
                    let value = args.get(name).and_then(Value::as_str).ok_or_else(invalid)?;
                    if value.len() > 4096 {
                        return Err(DomainError::LimitExceeded("MCP template value".into()));
                    }
                    expanded.push_str(literal);
                    for byte in value.bytes() {
                        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                            expanded.push(byte as char)
                        } else {
                            use std::fmt::Write;
                            write!(&mut expanded, "%{byte:02X}").map_err(|_| invalid())?;
                        }
                    }
                    if expanded.len() > 4096 {
                        return Err(DomainError::LimitExceeded("MCP expanded URI".into()));
                    }
                    rest = tail;
                }
                expanded.push_str(rest);
                uri(&expanded)?;
                json!({"uri":expanded})
            }
        })
    }
}
