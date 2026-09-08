use super::*;
fn compat(input: &RequestContext<'_>, key: &str, default: bool) -> bool {
    input.model["compat"][key].as_bool().unwrap_or(default)
}
fn content(value: &Value, mistral: bool) -> Value {
    if value.is_string() {
        return value.clone();
    }
    json!(
        blocks(value)
            .iter()
            .filter_map(|b| match string(b, "type") {
                "text" if !string(b, "text").trim().is_empty() =>
                    Some(json!({"type":"text","text":b["text"]})),
                "image" => Some(if mistral {
                    json!({"type":"image_url","image_url":image_url(b)})
                } else {
                    json!({"type":"image_url","image_url":{"url":image_url(b)}})
                }),
                _ => None,
            })
            .collect::<Vec<_>>()
    )
}
pub(super) fn build(
    input: &RequestContext<'_>,
    family: ApiFamily,
    messages: &[Value],
) -> Result<Value, DomainError> {
    let mistral = family == ApiFamily::Mistral;
    let model = input.model;
    let opts = input.options;
    let provider = string(model, "provider");
    let url = string(model, "baseUrl");
    let nvidia =
        ["nvidia", "nvidia-nim"].contains(&provider) || url.contains("integrate.api.nvidia.com");
    let zai = provider == "zai" || url.contains("api.z.ai");
    let nonstandard = ["cerebras", "xai", "zai", "opencode"].contains(&provider)
        || [
            "cerebras.ai",
            "api.x.ai",
            "chutes.ai",
            "deepseek.com",
            "opencode.ai",
        ]
        .iter()
        .any(|part| url.contains(part))
        || zai;
    let developer = !mistral
        && flag(model, "reasoning")
        && compat(input, "supportsDeveloperRole", !nonstandard && !nvidia);
    let mut converted = vec![];
    if !string(input.context, "systemPrompt").is_empty() {
        converted.push(json!({"role":if developer{"developer"}else{"system"},"content":input.context["systemPrompt"]}));
    }
    let mut index = 0;
    while index < messages.len() {
        let m = &messages[index];
        index += 1;
        match string(m, "role") {
            "user" => {
                let value = content(&m["content"], mistral);
                if value.as_array().is_none_or(|a| !a.is_empty()) {
                    converted.push(json!({"role":"user","content":value}));
                }
            }
            "assistant" => {
                let parts = blocks(&m["content"]);
                let texts: Vec<_> = parts
                    .iter()
                    .filter(|b| b["type"] == "text" && !string(b, "text").trim().is_empty())
                    .map(|b| string(b, "text"))
                    .collect();
                let mut value = json!({"role":"assistant","content":if texts.is_empty(){Value::Null}else{json!(texts.join(""))}});
                if mistral {
                    let parts:Vec<_>=parts.iter().filter_map(|b|match string(b,"type"){
      "text" if !string(b,"text").trim().is_empty()=>Some(json!({"type":"text","text":b["text"]})),
      "thinking" if !string(b,"thinking").trim().is_empty()=>Some(json!({"type":"thinking","thinking":[{"type":"text","text":b["thinking"]}],"closed":true})),_=>None}).collect();
                    value["content"] = json!(parts);
                    value["prefix"] = json!(false);
                } else {
                    for b in parts.iter().filter(|b| b["type"] == "thinking") {
                        let sig = string(b, "thinkingSignature");
                        if compat(input, "requiresThinkingAsText", false) {
                            let combined = [
                                string(b, "thinking"),
                                value["content"].as_str().unwrap_or(""),
                            ]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                            value["content"] = json!(combined);
                        } else if !sig.is_empty() && !string(b, "thinking").is_empty() {
                            if !["reasoning_content", "reasoning", "reasoning_text"].contains(&sig)
                            {
                                return Err(invalid("completion reasoning signature field"));
                            }
                            let old = value[sig].as_str().unwrap_or("");
                            value[sig] = json!(format!("{old}{}", string(b, "thinking")));
                        }
                    }
                }
                let mut calls = vec![];
                let mut reasoning = vec![];
                for b in parts.iter().filter(|b| b["type"] == "toolCall") {
                    let mut call = json!({"id":b["id"],"type":"function","function":{"name":b["name"],"arguments":serde_json::to_string(&b["arguments"]).map_err(|_|invalid("tool arguments"))?}});
                    if mistral {
                        call["index"] = json!(calls.len());
                    }
                    calls.push(call);
                    if !mistral && !string(b, "thoughtSignature").is_empty() {
                        let v: Value = serde_json::from_str(string(b, "thoughtSignature"))
                            .map_err(|_| invalid("tool thought signature"))?;
                        reasoning.push(v);
                    }
                }
                if !calls.is_empty() {
                    value["tool_calls"] = json!(calls);
                }
                if !reasoning.is_empty() {
                    value["reasoning_details"] = json!(reasoning);
                }
                if value["content"].is_null()
                    && compat(input, "requiresAssistantAfterToolResult", false)
                {
                    value["content"] = json!("");
                }
                if value.get("tool_calls").is_some()
                    || value["content"].as_str().is_some_and(|s| !s.is_empty())
                    || value["content"].as_array().is_some_and(|a| !a.is_empty())
                {
                    converted.push(value);
                }
            }
            "toolResult" => {
                if mistral {
                    let txt = text(&m["content"]);
                    let has_images = blocks(&m["content"]).iter().any(|b| b["type"] == "image");
                    let txt = if txt.trim().is_empty() {
                        if has_images {
                            "(see attached image)"
                        } else {
                            "(no tool output)"
                        }
                    } else {
                        txt.trim()
                    };
                    let txt = format!(
                        "{}{}",
                        if flag(m, "isError") {
                            "[tool error] "
                        } else {
                            ""
                        },
                        txt
                    );
                    let mut body = vec![json!({"type":"text","text":txt})];
                    body.extend(
                        content(&m["content"], true)
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|b| b["type"] == "image_url")
                            .cloned(),
                    );
                    converted.push(json!({"role":"tool","content":body,"tool_call_id":m["toolCallId"],"name":m["toolName"]}));
                    continue;
                }
                let mut images = vec![];
                let mut current = m;
                loop {
                    let parts = blocks(&current["content"]);
                    images.extend(parts.iter().filter(|b| b["type"] == "image").cloned());
                    let text = text(&current["content"]);
                    let mut value = json!({"role":"tool","tool_call_id":current["toolCallId"],"content":if text.is_empty()&&parts.iter().any(|b|b["type"]=="image"){"(see attached image)"}else{&text}});
                    if compat(input, "requiresToolResultName", false) {
                        value["name"] = current["toolName"].clone();
                    }
                    converted.push(value);
                    if index < messages.len() && messages[index]["role"] == "toolResult" {
                        current = &messages[index];
                        index += 1;
                    } else {
                        break;
                    }
                }
                if !images.is_empty() {
                    if compat(input, "requiresAssistantAfterToolResult", false) {
                        converted.push(json!({"role":"assistant","content":"I have processed the tool results."}));
                    }
                    let mut parts =
                        vec![json!({"type":"text","text":"Attached image(s) from tool result:"})];
                    parts.extend(
                        content(&json!(images), false)
                            .as_array()
                            .unwrap()
                            .iter()
                            .cloned(),
                    );
                    converted.push(json!({"role":"user","content":parts}));
                }
            }
            _ => unreachable!(),
        }
    }
    let mut out = json!({"model":model["id"],"messages":converted,"stream":true});
    if !mistral && compat(input, "supportsUsageInStreaming", true) {
        out["stream_options"] = json!({"include_usage":true});
    }
    if !mistral && compat(input, "supportsStore", !nonstandard) {
        out["store"] = json!(false);
    }
    let max_field = if mistral {
        "max_tokens"
    } else {
        model["compat"]["maxTokensField"]
            .as_str()
            .unwrap_or(if url.contains("chutes.ai") {
                "max_tokens"
            } else {
                "max_completion_tokens"
            })
    };
    if !["max_tokens", "max_completion_tokens"].contains(&max_field) {
        return Err(invalid("maximum token field"));
    }
    option(&mut out, opts, "maxTokens", max_field);
    option(&mut out, opts, "temperature", "temperature");
    if mistral {
        option(&mut out, opts, "promptMode", "prompt_mode");
    }
    if ["llamacpp", "llama.cpp"].contains(&provider) {
        out["cache_prompt"] = json!(true);
        out["temperature"] = opts.get("temperature").cloned().unwrap_or(json!(0.7));
        for (key, value) in [
            ("top_p", json!(0.95)),
            ("top_k", json!(20)),
            ("min_p", json!(0)),
            ("presence_penalty", json!(0)),
            ("repetition_penalty", json!(1)),
        ] {
            out[key] = value;
        }
        let cap = opts["maxTokens"]
            .as_u64()
            .filter(|v| *v > 0)
            .unwrap_or(65536)
            .min(65536);
        out["max_tokens"] = json!(cap);
        out["max_completion_tokens"] = json!(cap);
    }
    if let Some(defs) = tools(input)? {
        out["tools"]=json!(defs.iter().map(|t|{let mut f=json!({"name":t["name"],"description":t["description"],"parameters":t["parameters"]});if mistral||compat(input,"supportsStrictMode",true){f["strict"]=json!(false);}json!({"type":"function","function":f})}).collect::<Vec<_>>());
    } else if messages.iter().any(|m| {
        blocks(&m["content"])
            .iter()
            .any(|b| b["type"] == "toolCall")
    }) {
        out["tools"] = json!([]);
    }
    option(&mut out, opts, "toolChoice", "tool_choice");
    if !mistral {
        let format = model["compat"]["thinkingFormat"]
            .as_str()
            .unwrap_or(if zai {
                "zai"
            } else if provider == "openrouter" || url.contains("openrouter.ai") {
                "openrouter"
            } else {
                "openai"
            });
        let effort = string(opts, "reasoningEffort");
        let enabled = !effort.is_empty() && effort != "off";
        if flag(model, "reasoning") {
            match format {
                "qwen-chat-template" => {
                    out["chat_template_kwargs"] = json!({"enable_thinking":enabled});
                    if enabled {
                        let apply = string(&model["thinking"], "apply");
                        if apply == "thinking_budget_tokens" {
                            out["chat_template_kwargs"]["thinking_budget_tokens"] =
                                json!(match effort {
                                    "minimal" => 512,
                                    "low" => 2048,
                                    "medium" => 8192,
                                    "xhigh" => 32768,
                                    _ => 16384,
                                });
                        }
                        if apply == "reasoning_effort" {
                            let native = model["thinking"]["modes"]
                                .as_array()
                                .and_then(|a| a.iter().find(|m| m["id"] == effort))
                                .and_then(|v| v["nativeValue"].as_str())
                                .unwrap_or(effort);
                            out["chat_template_kwargs"]["reasoning_effort"] = json!(native);
                            out["reasoning_effort"] = json!(native);
                        }
                    }
                }
                "zai" | "qwen" => {
                    out["enable_thinking"] = json!(enabled);
                }
                "openrouter" => {
                    out["reasoning"] = json!({"effort":if enabled{effort}else{"none"}});
                }
                _ if enabled
                    && compat(
                        input,
                        "supportsReasoningEffort",
                        !nvidia && !zai && provider != "xai" && !url.contains("api.x.ai"),
                    ) =>
                {
                    out["reasoning_effort"] = json!(if (provider == "groq"
                        || url.contains("groq.com"))
                        && model["id"] == "qwen/qwen3-32b"
                    {
                        "default"
                    } else {
                        effort
                    });
                }
                _ => {}
            }
        }
        if url.contains("openrouter.ai")
            && let Some(routing) = model["compat"].get("openRouterRouting")
        {
            out["provider"] = routing.clone();
        }
        if url.contains("ai-gateway.vercel.sh") {
            let routing = &model["compat"]["vercelGatewayRouting"];
            let mut gateway = json!({});
            for key in ["only", "order"] {
                if let Some(v) = routing.get(key) {
                    gateway[key] = v.clone();
                }
            }
            if !gateway.as_object().unwrap().is_empty() {
                out["providerOptions"] = json!({"gateway":gateway});
            }
        }
        if compat(input, "zaiToolStream", false) && input.context.get("tools").is_some() {
            out["tool_stream"] = json!(true);
        }
        if let Some(effort) = out["reasoning_effort"].as_str().map(str::to_owned)
            && let Some(mapped) = model["compat"]["reasoningEffortMap"].get(&effort)
        {
            out["reasoning_effort"] = mapped.clone();
        }
    }
    Ok(out)
}
