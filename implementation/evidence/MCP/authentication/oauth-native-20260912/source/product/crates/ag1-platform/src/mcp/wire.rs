//! Bounded, duplicate-key-rejecting JSON intake shared by future transports.
use super::invalid;
use ag1_core::DomainError;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::{fmt, io::BufRead};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    bytes: usize,
    depth: usize,
    nodes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 1024 * 1024,
            depth: 32,
            nodes: 20_000,
        }
    }
}

impl Limits {
    /// Callers may lower limits, but cannot exceed the initial safe ceilings.
    pub fn new(bytes: usize, depth: usize, nodes: usize) -> Result<Self, DomainError> {
        let ceiling = Self::default();
        if bytes == 0
            || bytes > ceiling.bytes
            || depth == 0
            || depth > ceiling.depth
            || nodes == 0
            || nodes > ceiling.nodes
        {
            return Err(invalid("JSON limits"));
        }
        Ok(Self {
            bytes,
            depth,
            nodes,
        })
    }
}

struct Budget {
    remaining: usize,
    depth: usize,
}

struct Seed<'a> {
    budget: &'a mut Budget,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Value, D::Error> {
        if self.budget.remaining == 0 || self.depth > self.budget.depth {
            return Err(de::Error::custom("MCP JSON complexity limit"));
        }
        self.budget.remaining -= 1;
        decoder.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded MCP JSON")
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("nonfinite number"))
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(Seed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate MCP JSON key"));
            }
            let value = map.next_value_seed(Seed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

pub fn decode_json(bytes: &[u8], limits: Limits) -> Result<Value, DomainError> {
    if bytes.len() > limits.bytes {
        return Err(DomainError::LimitExceeded("MCP frame bytes".into()));
    }
    let mut budget = Budget {
        remaining: limits.nodes,
        depth: limits.depth,
    };
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Seed {
        budget: &mut budget,
        depth: 0,
    }
    .deserialize(&mut decoder)
    .map_err(|_| invalid("malformed, duplicate-key or over-complex JSON"))?;
    decoder
        .end()
        .map_err(|_| invalid("trailing JSON content"))?;
    Ok(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Request,
    Notification,
    Response,
    Error,
}

#[derive(Debug)]
pub struct Message {
    pub kind: MessageKind,
    /// Untrusted original decoded envelope. Does not confer AG1 authority.
    pub body: Value,
}

fn valid_id(id: &Value) -> bool {
    id.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256)
        || id
            .as_i64()
            .is_some_and(|n| (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&n))
}

pub fn decode_message(bytes: &[u8], limits: Limits) -> Result<Message, DomainError> {
    let body = decode_json(bytes, limits)?;
    let object = body
        .as_object()
        .ok_or_else(|| invalid("JSON-RPC object required; batches unsupported"))?;
    if body["jsonrpc"] != "2.0" {
        return Err(invalid("JSON-RPC version"));
    }
    let kind = if let Some(method) = object.get("method") {
        if !method
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 256)
            || object.contains_key("result")
            || object.contains_key("error")
            || object.get("params").is_some_and(|p| !p.is_object())
        {
            return Err(invalid("JSON-RPC request envelope"));
        }
        match object.get("id") {
            None => MessageKind::Notification,
            Some(id) if valid_id(id) => MessageKind::Request,
            _ => return Err(invalid("JSON-RPC request ID")),
        }
    } else {
        if object.contains_key("params")
            || object.contains_key("result") == object.contains_key("error")
        {
            return Err(invalid("JSON-RPC response envelope"));
        }
        if let Some(error) = object.get("error") {
            if !error.is_object()
                || error["code"].as_i64().is_none()
                || !error["message"].is_string()
                || !object
                    .get("id")
                    .is_some_and(|id| id.is_null() || valid_id(id))
            {
                return Err(invalid("JSON-RPC error envelope"));
            }
            MessageKind::Error
        } else {
            if !object.get("id").is_some_and(valid_id) {
                return Err(invalid("JSON-RPC response ID"));
            }
            MessageKind::Response
        }
    };
    Ok(Message { kind, body })
}

/// Codec only: use on an owned worker with transport deadlines, never the UI or
/// session owner thread. An invalid frame poisons the reader; no resynchronization
/// over arbitrary stdout is attempted. The byte ceiling excludes the LF delimiter.
pub struct StdioReader<R> {
    reader: R,
    limits: Limits,
    failed: bool,
}

impl<R: BufRead> StdioReader<R> {
    pub fn new(reader: R, limits: Limits) -> Self {
        Self {
            reader,
            limits,
            failed: false,
        }
    }

    pub fn read(&mut self) -> Result<Option<Message>, DomainError> {
        if self.failed {
            return Err(invalid("stdio reader retired"));
        }
        let result = self.read_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn read_inner(&mut self) -> Result<Option<Message>, DomainError> {
        let mut frame = Vec::new();
        loop {
            let bytes = self
                .reader
                .fill_buf()
                .map_err(|_| invalid("stdio read failed"))?;
            if bytes.is_empty() {
                return if frame.is_empty() {
                    Ok(None)
                } else {
                    Err(invalid("truncated stdio frame"))
                };
            }
            let newline = bytes.iter().position(|b| *b == b'\n');
            let take = newline.unwrap_or(bytes.len());
            if take > self.limits.bytes.saturating_sub(frame.len()) {
                return Err(DomainError::LimitExceeded("MCP frame bytes".into()));
            }
            frame.extend_from_slice(&bytes[..take]);
            self.reader.consume(take + usize::from(newline.is_some()));
            if newline.is_some() {
                return decode_message(&frame, self.limits).map(Some);
            }
        }
    }
}
