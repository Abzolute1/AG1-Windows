use super::*;
pub(super) fn finish(
    out: &mut Value,
    input: &RequestContext<'_>,
    family: ApiFamily,
) -> Result<(), DomainError> {
    let opts = input.options;
    let model = input.model;
    let id = string(model, "id");
    match family {
        ApiFamily::Responses | ApiFamily::AzureResponses | ApiFamily::CodexResponses => {
            let codex = family == ApiFamily::CodexResponses;
            if family == ApiFamily::Responses {
                if opts["cacheRetention"] == "none" {
                    out.as_object_mut().unwrap().remove("prompt_cache_key");
                }
                if opts["cacheRetention"] == "long"
                    && string(model, "baseUrl").contains("api.openai.com")
                {
                    out["prompt_cache_retention"] = json!("24h");
                }
                option(out, opts, "serviceTier", "service_tier");
            }
            if family == ApiFamily::AzureResponses {
                option(out, opts, "azureDeploymentName", "model");
            }
            let effort = string(opts, "reasoningEffort");
            out.as_object_mut().unwrap().remove("reasoning");
            if (codex && opts.get("reasoningEffort").is_some())
                || (!codex
                    && flag(model, "reasoning")
                    && (!effort.is_empty() || !string(opts, "reasoningSummary").is_empty()))
            {
                let mut effort = if effort.is_empty() {
                    "medium"
                } else if !codex && effort == "max" {
                    "xhigh"
                } else {
                    effort
                };
                if codex {
                    let id = id.rsplit('/').next().unwrap_or("");
                    if ["gpt-5.2", "gpt-5.3", "gpt-5.4", "gpt-5.6"]
                        .iter()
                        .any(|prefix| id.starts_with(prefix))
                        && effort == "minimal"
                    {
                        effort = "low";
                    }
                    if id == "gpt-5.1" && effort == "xhigh" {
                        effort = "high";
                    }
                    if id == "gpt-5.1-codex-mini" {
                        effort = if ["high", "xhigh"].contains(&effort) {
                            "high"
                        } else {
                            "medium"
                        };
                    }
                }
                out["reasoning"] = json!({"effort":effort,"summary":opts.get("reasoningSummary").unwrap_or(&json!("auto"))});
                out["include"] = json!(["reasoning.encrypted_content"]);
            } else if flag(model, "reasoning")
                && !codex
                && (family == ApiFamily::AzureResponses || model["provider"] != "github-copilot")
            {
                out["reasoning"] = json!({"effort":"none"});
            }
        }
        ApiFamily::Google | ApiFamily::GeminiCli | ApiFamily::Vertex => {
            let v = if family == ApiFamily::GeminiCli {
                &mut out["request"]
            } else {
                out
            };
            if family == ApiFamily::GeminiCli {
                option(v, opts, "sessionId", "sessionId");
            }
            v["generationConfig"]
                .as_object_mut()
                .unwrap()
                .remove("thinkingConfig");
            if flag(model, "reasoning") && opts.get("thinking").is_some() {
                let thinking = &opts["thinking"];
                let mut cfg = if flag(thinking, "enabled") {
                    json!({"includeThoughts":true})
                } else {
                    let id = id.to_lowercase();
                    if id.contains("gemini-3") && id.contains("pro") {
                        json!({"thinkingLevel":"LOW"})
                    } else if id.contains("gemini-3") && id.contains("flash") {
                        json!({"thinkingLevel":"MINIMAL"})
                    } else {
                        json!({"thinkingBudget":0})
                    }
                };
                if flag(thinking, "enabled") {
                    if let Some(level) = thinking.get("level") {
                        cfg["thinkingLevel"] = level.clone();
                    } else {
                        option(&mut cfg, thinking, "budgetTokens", "thinkingBudget");
                    }
                }
                v["generationConfig"]["thinkingConfig"] = cfg;
            }
        }
        ApiFamily::Bedrock => {
            if opts["toolChoice"] == "none" {
                out.as_object_mut().unwrap().remove("toolConfig");
            } else if opts["toolChoice"]["type"] == "tool" && out.get("toolConfig").is_some() {
                out["toolConfig"]["toolChoice"] =
                    json!({"tool":{"name":opts["toolChoice"]["name"]}});
            }
            let id = id.to_lowercase();
            let claude = id.contains("anthropic.claude") || id.contains("anthropic/claude");
            let caching = opts["cacheRetention"] != "none"
                && (flag(opts, "forceBedrockCache")
                    || id.contains("claude")
                        && (id.contains("-4-")
                            || id.contains("-4.")
                            || id.contains("claude-3-7-sonnet")
                            || id.contains("claude-3-5-haiku")));
            if caching {
                let mut point = json!({"cachePoint":{"type":"default"}});
                if opts["cacheRetention"] == "long" {
                    point["cachePoint"]["ttl"] = json!("1h");
                }
                if let Some(system) = out["system"].as_array_mut() {
                    system.push(point.clone());
                }
                if let Some(last) = out["messages"].as_array_mut().and_then(|a| a.last_mut())
                    && last["role"] == "user"
                {
                    last["content"].as_array_mut().unwrap().push(point);
                }
            }
            if claude && flag(model, "reasoning") && !string(opts, "reasoning").is_empty() {
                let level = string(opts, "reasoning");
                let adaptive = ["opus-4-6", "opus-4.6", "sonnet-4-6", "sonnet-4.6"]
                    .iter()
                    .any(|s| id.contains(s));
                let fields = if adaptive {
                    let effort = match level {
                        "minimal" | "low" => "low",
                        "medium" => "medium",
                        "max" if id.contains("opus") => "max",
                        _ => "high",
                    };
                    json!({"thinking":{"type":"adaptive"},"output_config":{"effort":effort}})
                } else {
                    let level = if ["xhigh", "max"].contains(&level) {
                        "high"
                    } else {
                        level
                    };
                    let budget = opts["thinkingBudgets"][level]
                        .as_u64()
                        .unwrap_or(match level {
                            "minimal" => 1024,
                            "low" => 2048,
                            "medium" => 8192,
                            _ => 16384,
                        });
                    let mut fields = json!({"thinking":{"type":"enabled","budget_tokens":budget}});
                    if opts["interleavedThinking"].as_bool().unwrap_or(true) {
                        fields["anthropic_beta"] = json!(["interleaved-thinking-2025-05-14"]);
                    }
                    fields
                };
                out["additionalModelRequestFields"] = fields;
            }
        }
        _ => {}
    }
    Ok(())
}
