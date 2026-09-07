//! Event-driven YAML frontmatter with explicit materialization and alias budgets.
use crate::DomainError;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use yaml_rust2::{
    parser::{Event, Parser, Tag},
    scanner::TScalarStyle,
    yaml::Yaml,
};
const MAX_NODES: usize = 20000;
const MAX_BYTES: usize = 4 * 1024 * 1024;
#[derive(Serialize)]
pub struct Frontmatter {
    pub frontmatter: Value,
    pub body: String,
}
#[derive(Clone)]
struct Node {
    value: Value,
    nodes: usize,
    bytes: usize,
    depth: usize,
}
struct Frame {
    value: Value,
    key: Option<String>,
    anchor: usize,
    nodes: usize,
    bytes: usize,
    depth: usize,
}
struct Budget {
    nodes: usize,
    bytes: usize,
}
impl Budget {
    fn charge(&mut self, nodes: usize, bytes: usize) -> Result<(), DomainError> {
        self.nodes = self.nodes.saturating_add(nodes);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.nodes > MAX_NODES || self.bytes > MAX_BYTES {
            return Err(DomainError::LimitExceeded("YAML materialization".into()));
        }
        Ok(())
    }
}
fn invalid(message: &str) -> DomainError {
    DomainError::InvalidInput(format!("YAML frontmatter: {message}"))
}
fn scalar(text: String, style: TScalarStyle, tag: Option<Tag>) -> Result<Value, DomainError> {
    if let Some(tag) = tag {
        if tag.handle != "tag:yaml.org,2002:" || tag.suffix == "str" {
            return Ok(Value::String(text));
        }
        match tag.suffix.as_str() {
            "null" => return Ok(Value::Null),
            "bool" => {
                return match text.as_str() {
                    "true" | "True" | "TRUE" => Ok(Value::Bool(true)),
                    "false" | "False" | "FALSE" => Ok(Value::Bool(false)),
                    _ => Err(invalid("invalid tagged boolean")),
                };
            }
            "int" | "float" => {}
            _ => return Ok(Value::String(text)),
        }
    } else if style != TScalarStyle::Plain {
        return Ok(Value::String(text));
    }
    if matches!(text.as_str(), "Null" | "NULL") {
        return Ok(Value::Null);
    }
    Ok(match Yaml::from_str(&text) {
        Yaml::Null => Value::Null,
        Yaml::Boolean(v) => Value::Bool(v),
        Yaml::Integer(v) => json!(v),
        Yaml::Real(v) => v
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        _ => Value::String(text),
    })
}
fn attach(node: Node, frames: &mut [Frame], root: &mut Option<Node>) -> Result<(), DomainError> {
    if let Some(frame) = frames.last_mut() {
        frame.nodes = frame.nodes.saturating_add(node.nodes);
        frame.bytes = frame.bytes.saturating_add(node.bytes);
        frame.depth = frame.depth.max(node.depth + 1);
        match &mut frame.value {
            Value::Array(values) => values.push(node.value),
            Value::Object(values) => {
                if let Some(key) = frame.key.take() {
                    if values.contains_key(&key) {
                        return Err(invalid("duplicate mapping key"));
                    }
                    values.insert(key, node.value);
                } else {
                    frame.key = Some(match node.value {
                        Value::String(s) => s,
                        Value::Null => "null".into(),
                        Value::Bool(b) => b.to_string(),
                        Value::Number(n) => n.to_string(),
                        _ => return Err(invalid("complex mapping keys are unsupported")),
                    });
                }
            }
            _ => unreachable!("container frame"),
        }
    } else if root.replace(node).is_some() {
        return Err(invalid("multiple document values"));
    }
    Ok(())
}
fn anchor(
    node: &Node,
    id: usize,
    anchors: &mut BTreeMap<usize, Node>,
    budget: &mut Budget,
) -> Result<(), DomainError> {
    if id != 0 {
        if anchors.len() >= 128 && !anchors.contains_key(&id) {
            return Err(DomainError::LimitExceeded("YAML anchors".into()));
        }
        budget.charge(node.nodes, node.bytes)?;
        anchors.insert(id, node.clone());
    }
    Ok(())
}
pub fn yaml(input: &str) -> Result<Value, DomainError> {
    if input.len() > 256 * 1024 {
        return Err(DomainError::LimitExceeded("YAML input".into()));
    }
    let mut parser = Parser::new_from_str(input);
    let mut frames: Vec<Frame> = vec![];
    let mut root = None;
    let mut anchors = BTreeMap::new();
    let mut budget = Budget { nodes: 0, bytes: 0 };
    let mut documents = 0;
    for _ in 0..50000 {
        let (event, _) = parser.next_token().map_err(|e| {
            invalid(&format!(
                "{} at line {}, column {} (byte {})",
                e.info(),
                e.marker().line(),
                e.marker().col() + 1,
                e.marker().index()
            ))
        })?;
        match event {
            Event::StreamStart | Event::Nothing => {}
            Event::DocumentStart => {
                documents += 1;
                if documents > 1 {
                    return Err(invalid("multiple YAML documents"));
                }
            }
            Event::DocumentEnd => {}
            Event::StreamEnd => {
                if !frames.is_empty() {
                    return Err(invalid("unfinished YAML container"));
                }
                return Ok(root.map(|n: Node| n.value).unwrap_or(Value::Null));
            }
            Event::Scalar(text, style, id, tag) => {
                budget.charge(1, text.len())?;
                let bytes = text.len();
                let node = Node {
                    value: scalar(text, style, tag)?,
                    nodes: 1,
                    bytes,
                    depth: 0,
                };
                anchor(&node, id, &mut anchors, &mut budget)?;
                attach(node, &mut frames, &mut root)?;
            }
            Event::Alias(id) => {
                let node = anchors
                    .get(&id)
                    .ok_or_else(|| invalid("cyclic or unresolved YAML alias"))?;
                if frames.len() + node.depth > 32 {
                    return Err(DomainError::LimitExceeded("YAML alias depth".into()));
                }
                budget.charge(node.nodes, node.bytes)?;
                attach(node.clone(), &mut frames, &mut root)?;
            }
            Event::SequenceStart(id, _) | Event::MappingStart(id, _) => {
                if frames.len() >= 32 {
                    return Err(DomainError::LimitExceeded("YAML nesting".into()));
                }
                budget.charge(1, 0)?;
                let array = matches!(event, Event::SequenceStart(..));
                frames.push(Frame {
                    value: if array {
                        Value::Array(vec![])
                    } else {
                        Value::Object(Map::new())
                    },
                    key: None,
                    anchor: id,
                    nodes: 1,
                    bytes: 0,
                    depth: 1,
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let frame = frames
                    .pop()
                    .ok_or_else(|| invalid("unexpected container end"))?;
                if frame.key.is_some() {
                    return Err(invalid("mapping value missing"));
                }
                let node = Node {
                    value: frame.value,
                    nodes: frame.nodes,
                    bytes: frame.bytes,
                    depth: frame.depth,
                };
                anchor(&node, frame.anchor, &mut anchors, &mut budget)?;
                attach(node, &mut frames, &mut root)?;
            }
        }
    }
    Err(DomainError::LimitExceeded("YAML parser events".into()))
}
pub fn parse(content: &str) -> Result<Frontmatter, DomainError> {
    if content.len() > 1024 * 1024 {
        return Err(DomainError::LimitExceeded("resource document".into()));
    }
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.starts_with("---")
        && let Some(end) = normalized[3..].find("\n---").map(|i| i + 3)
    {
        let yaml_input =
            String::from_utf16_lossy(&normalized[..end].encode_utf16().skip(4).collect::<Vec<_>>());
        let body = normalized[end + 4..]
            .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
            .to_string();
        let parsed = if yaml_input.is_empty() {
            json!({})
        } else {
            yaml(&yaml_input)?
        };
        return Ok(Frontmatter {
            frontmatter: if parsed.is_null() { json!({}) } else { parsed },
            body,
        });
    }
    Ok(Frontmatter {
        frontmatter: json!({}),
        body: normalized,
    })
}
