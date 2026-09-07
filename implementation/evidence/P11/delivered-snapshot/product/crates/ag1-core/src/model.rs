//! Measured/declared model capabilities, native thinking controls and probe policy.
//! Model names do not determine capability tiers. Runtime actions are host ports.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityProfile {
    pub tool_call_reliability: String,
    pub prefers_xml_fallback: bool,
    pub max_useful_tool_result_tokens: u64,
    pub max_plan_horizon_steps: u64,
    pub needs_strict_edit_schema: bool,
    pub needs_short_verifier_output: bool,
    pub supports_images: bool,
    pub supports_reasoning_budget: bool,
    pub recommended_compaction_policy: String,
}
pub fn infer_capabilities(
    context_window: Option<u64>,
    local: bool,
    images: bool,
    reasoning: bool,
) -> CapabilityProfile {
    let context = context_window.filter(|n| *n > 0).unwrap_or(128000);
    let cramped = context < 16384;
    let medium = context < 65536;
    CapabilityProfile {
        tool_call_reliability: if cramped { "low" } else { "high" }.into(),
        prefers_xml_fallback: local,
        max_useful_tool_result_tokens: if cramped {
            1200
        } else if medium {
            1800
        } else {
            2400
        },
        max_plan_horizon_steps: if cramped {
            2
        } else if medium {
            3
        } else {
            4
        },
        needs_strict_edit_schema: cramped || local,
        needs_short_verifier_output: cramped || local,
        supports_images: images,
        supports_reasoning_budget: reasoning,
        recommended_compaction_policy: if cramped {
            "local_small"
        } else if context < 128000 {
            "small"
        } else if context < 200000 {
            "standard"
        } else {
            "large"
        }
        .into(),
    }
}
pub fn generation_max_tokens(context_window: Option<u64>, declared: Option<u64>) -> u64 {
    let context = context_window.filter(|n| *n > 0).unwrap_or(128000);
    let declared = declared.filter(|n| *n > 0);
    let floor = if context < 16384 {
        4096
    } else if context < 128000 {
        8192
    } else if context < 200000 {
        32768
    } else {
        65536
    };
    match declared {
        Some(value) if !(value <= 8192 && context >= 128000) => value.min(context),
        _ => floor.min(context),
    }
}
impl CapabilityProfile {
    pub fn serialize_tools(&self) -> bool {
        self.tool_call_reliability == "low" || self.recommended_compaction_policy == "local_small"
    }
    pub fn volume_rails(&self) -> bool {
        ["local_small", "small"].contains(&self.recommended_compaction_policy.as_str())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingMode {
    pub id: String,
    pub label: String,
    pub description: String,
    pub store_as: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingProfile {
    pub modes: Vec<ThinkingMode>,
    pub apply: Vec<String>,
    pub default_id: String,
    pub source: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeclaredThinking {
    pub modes: Vec<String>,
    pub apply: Option<Vec<String>>,
    pub default: Option<String>,
}
const MODES: [&str; 8] = [
    "off", "on", "minimal", "low", "medium", "high", "xhigh", "max",
];
pub fn mode(id: &str) -> ThinkingMode {
    ThinkingMode {
        id: id.into(),
        label: id.into(),
        description: match id {
            "off" => "No thinking".into(),
            "on" => "Thinking on (model default depth)".into(),
            _ => format!("Native effort: {id}"),
        },
        store_as: match id {
            "off" | "none" => "off",
            "on" => "high",
            v if MODES.contains(&v) => v,
            _ => "high",
        }
        .into(),
    }
}
fn word(text: &str, needle: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| w == needle)
}
pub fn template_thinking(template: &str) -> Option<ThinkingProfile> {
    let enable = word(template, "enable_thinking");
    let effort = word(template, "reasoning_effort");
    if !enable && !effort {
        return None;
    }
    let mut ids = vec!["off".to_string()];
    let mut apply = vec![];
    if enable {
        apply.push("enable_thinking".into());
        apply.push("chat_template_kwargs".into());
    }
    if effort {
        apply.push("reasoning_effort".into());
        if !apply.iter().any(|v| v == "chat_template_kwargs") {
            apply.push("chat_template_kwargs".into());
        }
        for line in template
            .lines()
            .filter(|line| line.contains("reasoning_effort"))
        {
            let bytes = line.as_bytes();
            for (index, &byte) in bytes.iter().enumerate() {
                if !b"'\"`".contains(&byte) {
                    continue;
                }
                let tail = &line[index + 1..];
                let length = tail
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || b"_-".contains(b))
                    .count();
                if length > 0
                    && tail
                        .as_bytes()
                        .get(length)
                        .is_some_and(|b| b"'\"`".contains(b))
                {
                    let value = tail[..length].to_lowercase();
                    let value = if value == "none" { "off".into() } else { value };
                    if MODES.contains(&value.as_str()) && !ids.contains(&value) {
                        ids.push(value);
                    }
                }
            }
        }
    }
    if enable && ids.len() == 1 {
        ids.push("on".into());
    }
    let default = if ids.iter().any(|v| v == "xhigh") {
        "xhigh"
    } else if ids.iter().any(|v| v == "on") {
        "on"
    } else {
        "off"
    };
    ids.sort_by_key(|id| MODES.iter().position(|v| v == id).unwrap_or(50));
    Some(ThinkingProfile {
        modes: ids.iter().map(|id| mode(id)).collect(),
        apply,
        default_id: default.into(),
        source: "template".into(),
    })
}
pub fn resolve_thinking(
    reasoning: bool,
    declared: Option<&DeclaredThinking>,
    template: Option<&str>,
    local_llama_budget: bool,
) -> ThinkingProfile {
    if let Some(declared) = declared.filter(|d| !d.modes.is_empty()) {
        let ids: Vec<_> = declared.modes.iter().map(|s| s.to_lowercase()).collect();
        return ThinkingProfile {
            modes: ids.iter().map(|id| mode(id)).collect(),
            apply: declared
                .apply
                .clone()
                .unwrap_or(vec!["reasoning_effort".into()]),
            default_id: declared
                .default
                .as_ref()
                .map(|s| s.to_lowercase())
                .unwrap_or_else(|| {
                    ids.iter()
                        .find(|id| *id != "off")
                        .unwrap_or(&ids[0])
                        .clone()
                }),
            source: "declared".into(),
        };
    }
    if let Some(mut profile) = template.and_then(template_thinking) {
        if local_llama_budget
            && profile.apply.iter().any(|v| v == "enable_thinking")
            && !profile.apply.iter().any(|v| v == "reasoning_effort")
        {
            profile.modes = ["off", "minimal", "low", "medium", "high", "xhigh"]
                .iter()
                .map(|id| mode(id))
                .collect();
            profile.apply.push("thinking_budget_tokens".into());
            profile.default_id = "high".into();
        }
        return profile;
    }
    ThinkingProfile {
        modes: if reasoning {
            vec![mode("off"), mode("on")]
        } else {
            vec![mode("off")]
        },
        apply: if reasoning {
            vec!["enable_thinking".into(), "chat_template_kwargs".into()]
        } else {
            vec![]
        },
        default_id: if reasoning { "on" } else { "off" }.into(),
        source: "fallback".into(),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisionVerdict {
    Yes,
    No,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisionObservation {
    pub verdict: VisionVerdict,
    pub source: Option<String>,
    pub reason: Option<String>,
}
pub fn needs_vision_canary(introspection: Option<&VisionObservation>, authorized: bool) -> bool {
    authorized
        && introspection.is_none_or(|v| {
            v.verdict != VisionVerdict::No
                && !(v.source.as_deref() == Some("llamacpp") && v.verdict != VisionVerdict::Unknown)
        })
}
pub fn resolve_vision(
    introspection: Option<VisionObservation>,
    canary: Option<VisionObservation>,
) -> VisionObservation {
    if let Some(initial) = introspection {
        if initial.verdict == VisionVerdict::No
            || (initial.source.as_deref() == Some("llamacpp")
                && initial.verdict != VisionVerdict::Unknown)
        {
            return initial;
        }
        if initial.verdict == VisionVerdict::Yes {
            return match canary {
                Some(canary) if canary.verdict == VisionVerdict::No => canary,
                Some(canary) if canary.verdict == VisionVerdict::Yes => VisionObservation {
                    reason: Some("confirmed by canary probe".into()),
                    ..initial
                },
                Some(canary) => VisionObservation {
                    reason: Some(format!(
                        "canary confirmation inconclusive: {}",
                        canary.reason.as_deref().unwrap_or("no details")
                    )),
                    ..initial
                },
                None => initial,
            };
        }
        return canary.unwrap_or(initial);
    }
    canary.unwrap_or(VisionObservation {
        verdict: VisionVerdict::Unknown,
        source: None,
        reason: Some(
            "no introspection available and no credentials or local endpoint for canary probe"
                .into(),
        ),
    })
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelScope {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub runtime_fingerprint: String,
}
#[derive(Default)]
pub struct CapabilityCache {
    vision: BTreeMap<ModelScope, (VisionObservation, u64)>,
    thinking: BTreeMap<ModelScope, ThinkingProfile>,
}
impl CapabilityCache {
    pub fn put_vision(&mut self, scope: ModelScope, value: VisionObservation, now_ms: u64) {
        if value.verdict == VisionVerdict::Unknown {
            return;
        }
        if self.vision.len() >= 256 && !self.vision.contains_key(&scope) {
            self.vision.clear();
        }
        self.vision.insert(scope, (value, now_ms));
    }
    pub fn vision(
        &self,
        scope: &ModelScope,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Option<&VisionObservation> {
        let (entry, at) = self.vision.get(scope)?;
        let ttl = if entry.verdict == VisionVerdict::No {
            ttl_ms.min(900000)
        } else {
            ttl_ms
        };
        if now_ms < *at || now_ms - *at > ttl {
            None
        } else {
            Some(entry)
        }
    }
    pub fn put_thinking(&mut self, scope: ModelScope, value: ThinkingProfile) {
        if value.source == "fallback" {
            return;
        }
        if self.thinking.len() >= 256 && !self.thinking.contains_key(&scope) {
            self.thinking.clear();
        }
        self.thinking.insert(scope, value);
    }
    pub fn thinking(&self, scope: &ModelScope) -> Option<&ThinkingProfile> {
        self.thinking.get(scope)
    }
}
