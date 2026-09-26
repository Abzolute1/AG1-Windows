//! Bounded private JSON intake. Strings (including unknown fields and keys) have
//! no Debug/Serialize route and clear their owned bytes on drop. Serde's internal
//! escape scratch and HTTP library buffers are not claimed to be locked/wiped.
use super::{MAX_DOCUMENT_BYTES, invalid};
use ag1_core::{DomainError, SecretBytes};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;

pub(super) enum Value {
    String(SecretBytes),
    Number(u64),
    Object(Vec<(SecretBytes, Value)>),
    Other,
}
impl Value {
    pub fn take(&mut self, key: &[u8]) -> Option<Self> {
        let Self::Object(fields) = self else {
            return None;
        };
        fields
            .iter()
            .position(|(k, _)| k.expose() == key)
            .map(|i| fields.swap_remove(i).1)
    }
    pub fn text(&self) -> Result<&str, DomainError> {
        match self {
            Self::String(s) => {
                std::str::from_utf8(s.expose()).map_err(|_| invalid("OAuth response text"))
            }
            _ => Err(invalid("OAuth response field type")),
        }
    }
    pub fn secret(self) -> Result<SecretBytes, DomainError> {
        match self {
            Self::String(s) => Ok(s),
            _ => Err(invalid("OAuth response field type")),
        }
    }
}
struct Seed<'a> {
    remaining: &'a mut usize,
    depth: usize,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Value, D::Error> {
        if *self.remaining == 0 || self.depth > 16 {
            return Err(de::Error::custom("private JSON budget"));
        }
        *self.remaining -= 1;
        decoder.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("private JSON")
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Value, E> {
        Ok(Value::Other)
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(u64::try_from(v).map_or(Value::Other, Value::Number))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v))
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Value, E> {
        Ok(Value::Other)
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Other)
    }
    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Other)
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(SecretBytes::new(v.as_bytes().to_vec())))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(SecretBytes::new(v.into_bytes())))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        while let Some(value) = seq.next_element_seed(Seed {
            remaining: self.remaining,
            depth: self.depth + 1,
        })? {
            drop(value);
        }
        Ok(Value::Other)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut fields: Vec<(SecretBytes, Value)> = Vec::new();
        while let Some(key) = map.next_key_seed(Seed {
            remaining: self.remaining,
            depth: self.depth + 1,
        })? {
            let Value::String(key) = key else {
                return Err(de::Error::custom("private JSON key"));
            };
            if fields.iter().any(|(k, _)| k.expose() == key.expose()) {
                return Err(de::Error::custom("duplicate private JSON key"));
            }
            let value = map.next_value_seed(Seed {
                remaining: self.remaining,
                depth: self.depth + 1,
            })?;
            fields.push((key, value));
        }
        Ok(Value::Object(fields))
    }
}
pub(super) fn decode(bytes: &[u8]) -> Result<Value, DomainError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(DomainError::LimitExceeded(
            "MCP OAuth response bytes".into(),
        ));
    }
    let mut remaining = 4096;
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Seed {
        remaining: &mut remaining,
        depth: 0,
    }
    .deserialize(&mut decoder)
    .map_err(|_| invalid("malformed, duplicate-key or over-complex private OAuth response"))?;
    decoder
        .end()
        .map_err(|_| invalid("trailing private OAuth response"))?;
    if !matches!(value, Value::Object(_)) {
        return Err(invalid("OAuth response object required"));
    }
    Ok(value)
}
