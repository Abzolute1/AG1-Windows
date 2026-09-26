//! Bounded JSON-schema subset used by every pinned TypeBox tool definition.
//! Unsupported validation keywords reject registration rather than silently weakening validation.
use crate::DomainError;
use regex::Regex;
use serde_json::Value;
#[derive(Clone, Debug)]
pub struct Schema(Value);
fn bad(detail: &str) -> DomainError {
    DomainError::InvalidInput(format!("tool schema: {detail}"))
}
const ANNOTATIONS: &[&str] = &[
    "title",
    "description",
    "default",
    "examples",
    "$comment",
    "deprecated",
    "readOnly",
    "writeOnly",
];
const KEYWORDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "minItems",
    "maxItems",
    "uniqueItems",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "enum",
    "const",
    "anyOf",
    "allOf",
    "oneOf",
    "not",
    "pattern",
    "minProperties",
    "maxProperties",
];
impl Schema {
    pub fn parse(value: &Value) -> Result<Self, DomainError> {
        bounded(value, 256 * 1024, 32)?;
        if value.to_string().len() > 256 * 1024 {
            return Err(bad("definition exceeds 256 KiB"));
        }
        check(value, 0)?;
        Ok(Self(value.clone()))
    }
    /// AJV-compatible scalar coercion happens before policy and approval binding.
    pub fn coerce(&self, args: &Value) -> Result<Value, DomainError> {
        bounded(args, 8 * 1024 * 1024, 64)?;
        let mut args = args.clone();
        if normalize(&self.0, &mut args, 0)? {
            self.validate(&args)?;
            Ok(args)
        } else {
            Err(bad("arguments do not match registered definition"))
        }
    }
    pub fn value(&self) -> &Value {
        &self.0
    }
    pub fn validate(&self, args: &Value) -> Result<(), DomainError> {
        bounded(args, 8 * 1024 * 1024, 64)?;
        if args.to_string().len() > 8 * 1024 * 1024 {
            return Err(bad("arguments exceed 8 MiB"));
        }
        if accepts(&self.0, args, 0)? {
            Ok(())
        } else {
            Err(bad("arguments do not match registered definition"))
        }
    }
}
fn check(s: &Value, depth: usize) -> Result<(), DomainError> {
    if depth > 32 {
        return Err(bad("nesting limit"));
    }
    if s.is_boolean() {
        return Ok(());
    }
    let obj = s
        .as_object()
        .ok_or_else(|| bad("schema must be an object or boolean"))?;
    for (key, value) in obj {
        if ANNOTATIONS.contains(&key.as_str()) {
            continue;
        }
        if !KEYWORDS.contains(&key.as_str()) {
            return Err(DomainError::Unavailable(format!(
                "unsupported tool schema keyword {key}"
            )));
        }
        match key.as_str() {
            "type" => {
                let names: Vec<_> = if let Some(name) = value.as_str() {
                    vec![name]
                } else {
                    value
                        .as_array()
                        .ok_or_else(|| bad("type"))?
                        .iter()
                        .map(|v| v.as_str().ok_or_else(|| bad("type name")))
                        .collect::<Result<_, _>>()?
                };
                if names.is_empty()
                    || names.iter().any(|s| {
                        ![
                            "null", "boolean", "object", "array", "number", "integer", "string",
                        ]
                        .contains(s)
                    })
                {
                    return Err(bad("type name"));
                }
            }
            "properties" => {
                for child in value.as_object().ok_or_else(|| bad("properties"))?.values() {
                    check(child, depth + 1)?;
                }
            }
            "additionalProperties" | "items" | "not" => check(value, depth + 1)?,
            "anyOf" | "allOf" | "oneOf" => {
                let rows = value.as_array().ok_or_else(|| bad("schema alternatives"))?;
                if rows.is_empty() || rows.len() > 256 {
                    return Err(bad("schema alternative limit"));
                }
                for child in rows {
                    check(child, depth + 1)?;
                }
            }
            "required" => {
                let rows = value.as_array().ok_or_else(|| bad("required"))?;
                let mut seen = std::collections::BTreeSet::new();
                for v in rows {
                    if !seen.insert(v.as_str().ok_or_else(|| bad("required name"))?) {
                        return Err(bad("duplicate required name"));
                    }
                }
            }
            "enum" => {
                if value.as_array().is_none_or(Vec::is_empty) {
                    return Err(bad("enum"));
                }
            }
            "const" => {}
            "pattern" => {
                let pattern = value.as_str().ok_or_else(|| bad("pattern"))?;
                Regex::new(pattern).map_err(|_| {
                    DomainError::Unavailable("unsupported tool schema regex".into())
                })?;
            }
            "uniqueItems" => {
                if !value.is_boolean() {
                    return Err(bad("uniqueItems"));
                }
            }
            "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf" => {
                if value
                    .as_f64()
                    .is_none_or(|n| !n.is_finite() || key == "multipleOf" && n <= 0.)
                {
                    return Err(bad("numeric bound"));
                }
            }
            _ => {
                if value.as_u64().is_none() {
                    return Err(bad("nonnegative integer bound"));
                }
            }
        }
    }
    Ok(())
}
fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "null" => v.is_null(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "number" => v.is_number(),
        "integer" => v.as_f64().is_some_and(|n| n.fract() == 0.),
        "string" => v.is_string(),
        _ => false,
    }
}
fn range(s: &Value, lo: &str, hi: &str, n: usize) -> bool {
    s[lo].as_u64().is_none_or(|b| n as u64 >= b) && s[hi].as_u64().is_none_or(|b| n as u64 <= b)
}
fn accepts(s: &Value, v: &Value, depth: usize) -> Result<bool, DomainError> {
    if depth > 64 {
        return Err(bad("argument nesting limit"));
    }
    if let Some(b) = s.as_bool() {
        return Ok(b);
    }
    if let Some(t) = s.get("type") {
        let matches = if let Some(t) = t.as_str() {
            type_matches(t, v)
        } else {
            t.as_array()
                .unwrap()
                .iter()
                .any(|t| type_matches(t.as_str().unwrap(), v))
        };
        if !matches {
            return Ok(false);
        }
    }
    if s.get("const").is_some_and(|c| c != v)
        || s.get("enum")
            .is_some_and(|e| !e.as_array().unwrap().contains(v))
    {
        return Ok(false);
    }
    for name in ["anyOf", "allOf", "oneOf"] {
        if let Some(rows) = s[name].as_array() {
            let mut count = 0;
            for child in rows {
                count += usize::from(accepts(child, v, depth + 1)?);
            }
            if name == "anyOf" && count == 0
                || name == "allOf" && count != rows.len()
                || name == "oneOf" && count != 1
            {
                return Ok(false);
            }
        }
    }
    if let Some(child) = s.get("not")
        && accepts(child, v, depth + 1)?
    {
        return Ok(false);
    }
    if let Some(rows) = v.as_object() {
        if !range(s, "minProperties", "maxProperties", rows.len()) {
            return Ok(false);
        }
        if s["required"]
            .as_array()
            .is_some_and(|r| r.iter().any(|k| !rows.contains_key(k.as_str().unwrap())))
        {
            return Ok(false);
        }
        for (key, value) in rows {
            let child = s["properties"]
                .get(key)
                .or_else(|| s.get("additionalProperties"));
            if let Some(child) = child
                && !accepts(child, value, depth + 1)?
            {
                return Ok(false);
            }
        }
    }
    if let Some(rows) = v.as_array() {
        if !range(s, "minItems", "maxItems", rows.len()) {
            return Ok(false);
        }
        if s["uniqueItems"] == true {
            let mut seen = std::collections::BTreeSet::new();
            for item in rows {
                if !seen.insert(item.to_string()) {
                    return Ok(false);
                }
            }
        }
        if let Some(child) = s.get("items") {
            for value in rows {
                if !accepts(child, value, depth + 1)? {
                    return Ok(false);
                }
            }
        }
    }
    if let Some(text) = v.as_str() {
        if !range(s, "minLength", "maxLength", text.chars().count()) {
            return Ok(false);
        }
        if let Some(pattern) = s["pattern"].as_str()
            && !Regex::new(pattern)
                .map_err(|_| bad("pattern"))?
                .is_match(text)
        {
            return Ok(false);
        }
    }
    if let Some(n) = v.as_f64() {
        if s["minimum"].as_f64().is_some_and(|b| n < b)
            || s["maximum"].as_f64().is_some_and(|b| n > b)
            || s["exclusiveMinimum"].as_f64().is_some_and(|b| n <= b)
            || s["exclusiveMaximum"].as_f64().is_some_and(|b| n >= b)
        {
            return Ok(false);
        }
        if let Some(b) = s["multipleOf"].as_f64() {
            let ratio = n / b;
            if (ratio - ratio.round()).abs() > f64::EPSILON * ratio.abs().max(1.) * 4. {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(crate) fn bounded(value: &Value, cap: usize, max_depth: usize) -> Result<(), DomainError> {
    let mut stack = vec![(value, 0)];
    let mut size = 0usize;
    let mut nodes = 0;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > 100000 || depth > max_depth {
            return Err(bad("JSON complexity limit"));
        }
        match value {
            Value::String(s) => size = size.saturating_add(s.len()),
            Value::Array(rows) => {
                if rows.len() > 100000usize.saturating_sub(nodes + stack.len()) {
                    return Err(bad("JSON complexity limit"));
                }
                stack.extend(rows.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(rows) => {
                if rows.len() > 100000usize.saturating_sub(nodes + stack.len()) {
                    return Err(bad("JSON complexity limit"));
                }
                for (k, v) in rows {
                    size = size.saturating_add(k.len());
                    stack.push((v, depth + 1));
                }
            }
            _ => size = size.saturating_add(8),
        }
        if size > cap {
            return Err(bad("JSON size limit"));
        }
    }
    Ok(())
}
fn convert(kind: &str, value: &Value) -> Option<Value> {
    match kind {
        "string" => match value {
            Value::Null => Some(Value::String(String::new())),
            Value::Bool(v) => Some(Value::String(v.to_string())),
            Value::Number(v) => Some(Value::String(v.to_string())),
            _ => None,
        },
        "number" | "integer" => {
            let n = match value {
                Value::Null => Some(0.),
                Value::Bool(v) => Some(if *v { 1. } else { 0. }),
                Value::String(s) if !s.trim().is_empty() => {
                    let s = s.trim();
                    if s.starts_with("0x") || s.starts_with("0X") {
                        u64::from_str_radix(&s[2..], 16).ok().map(|n| n as f64)
                    } else {
                        s.parse::<f64>().ok()
                    }
                }
                _ => None,
            }?;
            if !n.is_finite() || kind == "integer" && n.fract() != 0. {
                return None;
            }
            if n >= i64::MIN as f64 && n < i64::MAX as f64 && n.fract() == 0. {
                Some((n as i64).into())
            } else {
                serde_json::Number::from_f64(n).map(Value::Number)
            }
        }
        "boolean" => match value {
            Value::Null => Some(false.into()),
            Value::String(s) if s == "true" => Some(true.into()),
            Value::String(s) if s == "false" => Some(false.into()),
            Value::Number(n) if n.as_f64() == Some(0.) => Some(false.into()),
            Value::Number(n) if n.as_f64() == Some(1.) => Some(true.into()),
            _ => None,
        },
        "null" => {
            if value == &Value::Bool(false)
                || value.as_str() == Some("")
                || value.as_f64() == Some(0.)
            {
                Some(Value::Null)
            } else {
                None
            }
        }
        _ => None,
    }
}
fn normalize(s: &Value, v: &mut Value, depth: usize) -> Result<bool, DomainError> {
    if depth > 64 {
        return Err(bad("coercion nesting limit"));
    }
    if let Some(b) = s.as_bool() {
        return Ok(b);
    }
    if let Some(t) = s.get("type") {
        let types: Vec<_> = if let Some(t) = t.as_str() {
            vec![t]
        } else {
            t.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect()
        };
        if !types.iter().any(|t| type_matches(t, v)) {
            let Some(next) = types.iter().find_map(|t| convert(t, v)) else {
                return Ok(false);
            };
            *v = next;
        }
    }
    // AJV applies alternatives in declaration order and may coerce the same property more than once.
    for name in ["anyOf", "oneOf", "allOf"] {
        if let Some(rows) = s[name].as_array() {
            let mut matches = 0;
            for child in rows {
                if normalize(child, v, depth + 1)? {
                    matches += 1;
                    if name == "anyOf" {
                        break;
                    }
                }
            }
            if name == "anyOf" && matches == 0
                || name == "oneOf" && matches != 1
                || name == "allOf" && matches != rows.len()
            {
                return Ok(false);
            }
        }
    }
    if let Some(rows) = v.as_object_mut() {
        for (k, value) in rows {
            if let Some(child) = s["properties"]
                .get(k)
                .or_else(|| s.get("additionalProperties"))
                && !normalize(child, value, depth + 1)?
            {
                return Ok(false);
            }
        }
    }
    if let Some(rows) = v.as_array_mut()
        && let Some(child) = s.get("items")
    {
        for value in rows {
            if !normalize(child, value, depth + 1)? {
                return Ok(false);
            }
        }
    }
    accepts(s, v, depth)
}
