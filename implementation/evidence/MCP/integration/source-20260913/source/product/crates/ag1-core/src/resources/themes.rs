//! Data-only validation and variable resolution for the reference theme schema.
//! Terminal renderers and desktop appearance remain consumers of this data.
use crate::DomainError;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
pub const REQUIRED: &[&str] = &[
    "accent",
    "border",
    "borderAccent",
    "borderMuted",
    "success",
    "error",
    "warning",
    "muted",
    "dim",
    "text",
    "thinkingText",
    "accentShimmer",
    "successShimmer",
    "bashModeShimmer",
    "selectedBg",
    "userMessageBg",
    "userMessageText",
    "customMessageBg",
    "customMessageText",
    "customMessageLabel",
    "toolPendingBg",
    "toolSuccessBg",
    "toolErrorBg",
    "toolTitle",
    "toolOutput",
    "mdHeading",
    "mdLink",
    "mdLinkUrl",
    "mdCode",
    "mdCodeBlock",
    "mdCodeBlockBorder",
    "mdQuote",
    "mdQuoteBorder",
    "mdHr",
    "mdListBullet",
    "toolDiffAdded",
    "toolDiffRemoved",
    "toolDiffContext",
    "syntaxComment",
    "syntaxKeyword",
    "syntaxFunction",
    "syntaxVariable",
    "syntaxString",
    "syntaxNumber",
    "syntaxType",
    "syntaxOperator",
    "syntaxPunctuation",
    "thinkingOff",
    "thinkingMinimal",
    "thinkingLow",
    "thinkingMedium",
    "thinkingHigh",
    "thinkingXhigh",
    "bashMode",
];
fn invalid(reason: &str) -> DomainError {
    DomainError::InvalidInput(format!("theme {reason}"))
}
fn color(value: &Value) -> bool {
    value.as_str().is_some_and(|s| s.len() <= 8192)
        || value
            .as_f64()
            .is_some_and(|n| (0.0..=255.0).contains(&n) && n.fract() == 0.0)
}
fn resolve(value: &Value, vars: &Map<String, Value>) -> Result<Value, DomainError> {
    let mut value = value;
    let mut seen = BTreeSet::new();
    loop {
        if !color(value) {
            return Err(invalid("color value"));
        }
        let Some(name) = value.as_str() else {
            return Ok(value.clone());
        };
        if name.is_empty() {
            return Ok(value.clone());
        }
        if let Some(hex) = name.strip_prefix('#') {
            // The JS renderer uses parseInt on each pair (accepting e.g. #1x2233).
            // Preserve that selected source behavior; this module does not emit ANSI.
            if hex.len() != 6
                || !hex.is_ascii()
                || (0..6)
                    .step_by(2)
                    .any(|i| !hex.as_bytes()[i].is_ascii_hexdigit())
            {
                return Err(invalid("hex color"));
            }
            return Ok(value.clone());
        }
        if seen.len() >= 128 {
            return Err(DomainError::LimitExceeded("theme variable chain".into()));
        }
        if !seen.insert(name) {
            return Err(invalid("circular variable reference"));
        }
        value = vars
            .get(name)
            .ok_or_else(|| invalid("variable reference not found"))?;
    }
}
pub struct ValidatedTheme {
    pub name: String,
    pub raw: Value,
    pub colors: Map<String, Value>,
}
pub fn parse(text: &str) -> Result<ValidatedTheme, DomainError> {
    if text.len() > 1024 * 1024 {
        return Err(DomainError::LimitExceeded("theme bytes".into()));
    }
    let raw: Value = serde_json::from_str(text).map_err(|_| invalid("JSON"))?;
    let object = raw.as_object().ok_or_else(|| invalid("object"))?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("name"))?
        .to_owned();
    if name.len() > 8192 {
        return Err(DomainError::LimitExceeded("theme name".into()));
    }
    if object.get("$schema").is_some_and(|v| !v.is_string()) {
        return Err(invalid("schema URI"));
    }
    let empty = Map::new();
    let vars = match object.get("vars") {
        Some(v) => v.as_object().ok_or_else(|| invalid("variables"))?,
        None => &empty,
    };
    if vars.len() > 4096 {
        return Err(DomainError::LimitExceeded("theme variables".into()));
    }
    if vars.iter().any(|(k, v)| k.len() > 8192 || !color(v)) {
        return Err(invalid("variable value"));
    }
    if let Some(export) = object.get("export") {
        let export = export.as_object().ok_or_else(|| invalid("export"))?;
        for key in ["pageBg", "cardBg", "infoBg"] {
            if export.get(key).is_some_and(|v| !color(v)) {
                return Err(invalid("export color"));
            }
        }
    }
    let colors = object
        .get("colors")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("colors"))?;
    if colors.len() > 4096 {
        return Err(DomainError::LimitExceeded("theme colors".into()));
    }
    if REQUIRED.iter().any(|k| !colors.contains_key(*k)) {
        return Err(invalid("missing required colors"));
    }
    let mut resolved = Map::new();
    for (key, value) in colors {
        resolved.insert(key.clone(), resolve(value, vars)?);
    }
    for (key, fallback) in [
        ("toolDiffLineNum", "dim"),
        ("toolDiffAddedBg", "toolSuccessBg"),
        ("toolDiffRemovedBg", "toolErrorBg"),
    ] {
        if !resolved.contains_key(key) {
            resolved.insert(key.into(), resolved[fallback].clone());
        }
    }
    Ok(ValidatedTheme {
        name,
        raw,
        colors: resolved,
    })
}
