//! Display projections cannot turn server bytes into native artifact grants.
use ag1_core::DomainError;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct Projection {
    attempts: usize,
    encoded_bytes: usize,
}
impl Projection {
    pub fn image(&mut self, data: &Value, mime: &Value) -> Result<Vec<Value>, DomainError> {
        let data = data.as_str().ok_or_else(|| super::invalid("image data"))?;
        let mime = mime.as_str().ok_or_else(|| super::invalid("image MIME"))?;
        if mime.len() > 256 || mime.chars().any(char::is_control) || !mime.starts_with("image/") {
            return Err(super::invalid("image MIME"));
        }
        self.attempts += 1;
        let reason = if self.attempts > 8 {
            Some("the result exceeds the eight-image preview limit")
        } else if !matches!(
            mime,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ) {
            Some("this image format is unsupported")
        } else if data.len() > 8 * 1024 * 1024 {
            Some("the encoded image exceeds the preview byte limit")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Ok(vec![omitted(reason)]);
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| super::invalid("image base64"))?;
        match crate::image_tools::mcp_preview(&bytes, mime) {
            Ok(preview) => {
                let blocks = preview["content"]
                    .as_array()
                    .ok_or_else(|| super::invalid("native image preview"))?;
                let size: usize = blocks
                    .iter()
                    .filter_map(|b| b["data"].as_str())
                    .map(str::len)
                    .sum();
                if self.encoded_bytes.saturating_add(size) > 8 * 1024 * 1024 {
                    return Ok(vec![omitted(
                        "the result exceeds the aggregate preview byte limit",
                    )]);
                }
                self.encoded_bytes += size;
                let mut result = vec![
                    json!({"type":"text","text":"MCP image preview (rasterized; original bytes retained separately):"}),
                ];
                // Keep native size/resize notes, never server-supplied details.
                result.extend(blocks.iter().cloned());
                Ok(result)
            }
            Err(DomainError::QueueFull) => Ok(vec![omitted("the image decoder is busy")]),
            Err(DomainError::LimitExceeded(_)) => Ok(vec![omitted(
                "the image exceeds the decoder byte or pixel limit",
            )]),
            Err(_) => Ok(vec![omitted(
                "the image is malformed or its MIME type does not match its bytes",
            )]),
        }
    }
}
fn omitted(reason: &str) -> Value {
    json!({"type":"text","text":format!("MCP image preview unavailable: {reason}. Original encoded bytes remain in the retained MCP response.")})
}

/// Small literal cards; full names, URIs and all rows remain in original custody.
pub(super) fn references(envelope: &Value, kind: ag1_core::tools::mcp::Kind) -> Vec<Value> {
    let result = &envelope["result"];
    let rows: Vec<&Value> = match kind {
        ag1_core::tools::mcp::Kind::Tool => {
            result["content"].as_array().into_iter().flatten().collect()
        }
        ag1_core::tools::mcp::Kind::Prompt => result["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| &m["content"])
            .collect(),
        _ => result["contents"]
            .as_array()
            .into_iter()
            .flatten()
            .collect(),
    };
    fn label(value: &Value, limit: usize) -> String {
        let text = value.as_str().unwrap_or("");
        let mut chars = text.chars();
        let mut clipped: String = chars.by_ref().take(limit).collect();
        if chars.next().is_some() {
            clipped.push('…');
        }
        clipped
    }
    rows.into_iter().filter_map(|row| {
        let (resource, retained) = if row["type"]=="resource_link" {(row,false)}
            else if row["type"]=="resource" {(&row["resource"],true)}
            else if row["uri"].is_string() {(row,true)} else {return None};
        Some(json!({"name":label(&resource["name"],96),"uri":label(&resource["uri"],256),"retained":retained}))
    }).take(8).collect()
}
