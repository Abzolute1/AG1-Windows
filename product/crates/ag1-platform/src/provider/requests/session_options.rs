//! Resolve the selected session thinking level into provider-specific options.
//! Mirrors the pinned streamSimple adapters; backend profiles are never changed.
use super::*;

pub fn session_options(
    model: &Value,
    thinking: &str,
    overrides: Option<&Value>,
) -> Result<Value, DomainError> {
    if !["off", "minimal", "low", "medium", "high", "xhigh", "max"].contains(&thinking) {
        return Err(invalid("session thinking level"));
    }
    let family: ApiFamily =
        serde_json::from_value(model["api"].clone()).map_err(|_| invalid("provider family"))?;
    let mut out = overrides
        .cloned()
        .unwrap_or_else(|| json!({"maxTokens":model["maxTokens"]}));
    if !out.is_object() {
        return Err(invalid("host provider options"));
    }
    if out.get("maxTokens").is_none() {
        out["maxTokens"] = model["maxTokens"].clone();
    }
    let enabled = thinking != "off";
    let level = if ["xhigh", "max"].contains(&thinking) {
        "high"
    } else {
        thinking
    };
    let id = string(model, "id");
    let opus = id.contains("opus-4-6") || id.contains("opus-4.6");
    let adaptive = opus || id.contains("sonnet-4-6") || id.contains("sonnet-4.6");
    let xhigh = opus
        || ["gpt-5.2", "gpt-5.3", "gpt-5.4", "gpt-5.6"]
            .iter()
            .any(|s| id.contains(s))
        || model["thinking"]["modes"].as_array().is_some_and(|modes| {
            modes
                .iter()
                .any(|m| m.as_str().is_some_and(|s| s.eq_ignore_ascii_case("xhigh")))
        });
    let effort = match thinking {
        "minimal" => "low",
        "xhigh" | "max" if opus => "max",
        "xhigh" | "max" => "high",
        other => other,
    };
    match family {
        ApiFamily::Completions
        | ApiFamily::Responses
        | ApiFamily::AzureResponses
        | ApiFamily::CodexResponses => {
            if enabled {
                out["reasoningEffort"] = json!(if xhigh {
                    if thinking == "max"
                        && matches!(family, ApiFamily::Responses | ApiFamily::AzureResponses)
                    {
                        "xhigh"
                    } else {
                        thinking
                    }
                } else {
                    level
                });
            }
        }
        ApiFamily::Anthropic => {
            out["thinkingEnabled"] = json!(enabled);
            if enabled {
                if adaptive {
                    out["effort"] = json!(effort);
                } else {
                    let (maximum, budget) = adjust(&out, model, level)?;
                    out["maxTokens"] = json!(maximum);
                    out["thinkingBudgetTokens"] = json!(budget);
                }
            }
        }
        ApiFamily::Google | ApiFamily::Vertex | ApiFamily::GeminiCli => {
            out["thinking"] = json!({"enabled":enabled});
            if enabled {
                let lower = id.to_ascii_lowercase();
                let pro = gemini3(&lower, "pro");
                let flash = gemini3(&lower, "flash");
                if pro || flash {
                    out["thinking"]["level"] = json!(if pro {
                        if ["minimal", "low"].contains(&level) {
                            "LOW"
                        } else {
                            "HIGH"
                        }
                    } else {
                        match level {
                            "minimal" => "MINIMAL",
                            "low" => "LOW",
                            "medium" => "MEDIUM",
                            _ => "HIGH",
                        }
                    });
                } else if family == ApiFamily::GeminiCli {
                    let (maximum, budget) = adjust(&out, model, level)?;
                    out["maxTokens"] = json!(maximum);
                    out["thinking"]["budgetTokens"] = json!(budget);
                } else {
                    out["thinking"]["budgetTokens"] =
                        json!(if id.contains("2.5-pro") || id.contains("2.5-flash") {
                            match level {
                                "minimal" => 128,
                                "low" => 2048,
                                "medium" => 8192,
                                _ => {
                                    if id.contains("2.5-pro") {
                                        32768
                                    } else {
                                        24576
                                    }
                                }
                            }
                        } else {
                            -1
                        });
                }
            }
        }
        ApiFamily::Mistral => {
            if enabled && flag(model, "reasoning") {
                out["promptMode"] = json!("reasoning");
            }
        }
        ApiFamily::Bedrock => {
            if enabled
                && flag(model, "reasoning")
                && (id.contains("anthropic.claude") || id.contains("anthropic/claude"))
            {
                if adaptive {
                    out["additionalModelRequestFields"] =
                        json!({"thinking":{"type":"adaptive"},"output_config":{"effort":effort}});
                } else {
                    let (maximum, budget) = adjust(&out, model, level)?;
                    out["maxTokens"] = json!(maximum);
                    out["additionalModelRequestFields"] = json!({"thinking":{"type":"enabled","budget_tokens":budget},"anthropic_beta":["interleaved-thinking-2025-05-14"]});
                }
            }
        }
    }
    Ok(out)
}
fn gemini3(id: &str, suffix: &str) -> bool {
    // Match the pinned /gemini-3(?:\.\d+)?-(pro|flash)/ classification.
    let pattern = format!(r"gemini-3(?:\.\d+)?-{suffix}");
    regex::Regex::new(&pattern)
        .expect("fixed provider pattern")
        .is_match(id)
}
fn adjust(options: &Value, model: &Value, level: &str) -> Result<(u64, u64), DomainError> {
    let base = options["maxTokens"]
        .as_u64()
        .ok_or_else(|| invalid("host output token budget"))?;
    let cap = model["maxTokens"]
        .as_u64()
        .ok_or_else(|| invalid("model output token budget"))?;
    let budget = match level {
        "minimal" => 1024,
        "low" => 2048,
        "medium" => 8192,
        _ => 16384,
    };
    let maximum = base.saturating_add(budget).min(cap);
    Ok((
        maximum,
        if maximum <= budget {
            maximum.saturating_sub(1024)
        } else {
            budget
        },
    ))
}
