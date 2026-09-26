//! Pinned turn-health diagnostics. This analysis never cancels a generation,
//! limits reasoning, changes tool policy, or grants continuation authority.
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub severity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub output_tokens: u64,
    pub text_length: usize,
    pub reasons: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_hint: Option<&'static str>,
}
pub fn analyze(message: &Value, volume_rails: bool) -> Analysis {
    let content = &message["content"];
    let text_length = content
        .as_str()
        .map(|s| s.encode_utf16().count())
        .unwrap_or_else(|| {
            content
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .map(|s| s.encode_utf16().count())
                .sum()
        });
    let mut result = Analysis {
        severity: "normal",
        stop_reason: message["stopReason"].as_str().map(str::to_owned),
        output_tokens: message["usage"]["output"].as_u64().unwrap_or(0),
        text_length,
        reasons: vec![],
        recovery_hint: None,
    };
    let reason = match result.stop_reason.as_deref() {
        Some("error") if super::retry::classify(message).is_some() => return result,
        Some("error") => Some("error_turn"),
        Some("aborted") => Some("aborted_turn"),
        Some("max_tokens" | "length") => Some("truncated_turn"),
        _ => None,
    };
    if let Some(reason) = reason {
        result.severity = "critical";
        result.reasons.push(reason);
    }
    if volume_rails {
        // Tool arguments are work product, not verbose prose. Match the SDK's
        // rough four UTF-16 code units per token and round only the total.
        let chars: usize = content
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "toolCall")
            .filter_map(|b| b.get("arguments"))
            .filter(|a| a.is_object() || a.is_array())
            .map(|a| a.to_string().encode_utf16().count())
            .sum();
        let tokens = result
            .output_tokens
            .saturating_sub(chars.div_ceil(4) as u64);
        if tokens >= 14_000 {
            result.severity = "critical";
            result.reasons.push("very_high_output_tokens");
        } else if tokens >= 8_000 {
            if result.severity == "normal" {
                result.severity = "elevated";
            }
            result.reasons.push("high_output_tokens");
        }
        if text_length >= 48_000 {
            result.severity = "critical";
            result.reasons.push("very_long_output_text");
        } else if text_length >= 24_000 {
            if result.severity == "normal" {
                result.severity = "elevated";
            }
            result.reasons.push("long_output_text");
        }
    }
    result.recovery_hint = match result.severity {
        "normal" => None,
        "critical" if result.reasons.contains(&"truncated_turn") => Some(
            "Continue from the last concrete point only, keep the next response short, and avoid repeating prior analysis.",
        ),
        "critical"
            if result
                .reasons
                .iter()
                .any(|r| matches!(*r, "error_turn" | "aborted_turn")) =>
        {
            Some(
                "Recover from the failed turn directly. Keep the next response concise and avoid replaying the full prior reasoning.",
            )
        }
        "critical" => Some(
            "The last turn degraded. Continue tersely from the latest concrete state and avoid repeating earlier analysis.",
        ),
        _ => Some(
            "The last turn is getting too verbose. Keep the next response concise and avoid rehashing earlier analysis.",
        ),
    };
    result
}
