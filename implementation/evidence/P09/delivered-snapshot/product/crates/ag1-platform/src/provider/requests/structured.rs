use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
fn user_parts(value: &Value, family: ApiFamily) -> Result<Vec<Value>, DomainError> {
    blocks(value).iter().filter(|b| b["type"]=="image" || !string(b,"text").trim().is_empty()).map(|b|{
  let image=b["type"]=="image";
  Ok(match family {
   ApiFamily::Anthropic=>if image{json!({"type":"image","source":{"type":"base64","media_type":b["mimeType"],"data":b["data"]}})}else{json!({"type":"text","text":b["text"]})},
   ApiFamily::Responses|ApiFamily::AzureResponses|ApiFamily::CodexResponses=>if image{json!({"type":"input_image","detail":"auto","image_url":image_url(b)})}else{json!({"type":"input_text","text":b["text"]})},
   ApiFamily::Bedrock=>if image{let format=string(b,"mimeType").strip_prefix("image/").ok_or_else(||invalid("Bedrock image MIME"))?;if !["png","jpeg","gif","webp"].contains(&format){return Err(invalid("Bedrock image format"));}STANDARD.decode(string(b,"data")).map_err(|_|invalid("image base64"))?;json!({"image":{"source":{"bytes":b["data"]},"format":format}})}else{json!({"text":b["text"]})},
   _=>if image{json!({"inlineData":{"mimeType":b["mimeType"],"data":b["data"]}})}else{json!({"text":b["text"]})},
  })
 }).collect()
}
fn claude_name(name: &str, oauth: bool) -> String {
    if !oauth {
        return name.into();
    }
    const NAMES: &[&str] = &[
        "Read",
        "Write",
        "Edit",
        "Bash",
        "Grep",
        "Glob",
        "AskUserQuestion",
        "EnterPlanMode",
        "ExitPlanMode",
        "KillShell",
        "NotebookEdit",
        "Skill",
        "Task",
        "TaskOutput",
        "TodoWrite",
        "WebFetch",
        "WebSearch",
    ];
    NAMES
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
        .copied()
        .unwrap_or(name)
        .into()
}
fn cache(input: &RequestContext<'_>) -> Option<Value> {
    if input.options["cacheRetention"] == "none" {
        return None;
    }
    let mut value = json!({"type":"ephemeral"});
    if input.options["cacheRetention"] == "long"
        && string(input.model, "baseUrl").contains("api.anthropic.com")
    {
        value["ttl"] = json!("1h");
    }
    Some(value)
}
fn append_group(out: &mut Vec<Value>, part: Value, key: &str, marker: &str) {
    if let Some(last) = out.last_mut()
        && last["role"] == "user"
        && last[key].as_array().is_some_and(|a| {
            a.iter()
                .any(|p| p.get(marker).is_some() || p["type"] == marker)
        })
    {
        last[key].as_array_mut().unwrap().push(part);
    } else {
        let mut value = json!({"role":"user"});
        value[key] = json!([part]);
        out.push(value);
    }
}
fn valid_signature(s: &str) -> bool {
    !s.is_empty() && s.len().is_multiple_of(4) && STANDARD.decode(s).is_ok()
}
fn google_multimodal(model: &Value) -> bool {
    let id = string(model, "id").to_lowercase();
    let suffix = id
        .strip_prefix("gemini-live-")
        .or_else(|| id.strip_prefix("gemini-"));
    suffix
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|s| s.parse::<u64>().ok())
        .is_none_or(|major| major >= 3)
}
fn convert(
    input: &RequestContext<'_>,
    family: ApiFamily,
    messages: &[Value],
) -> Result<Vec<Value>, DomainError> {
    let mut out = vec![];
    let response = matches!(
        family,
        ApiFamily::Responses | ApiFamily::AzureResponses | ApiFamily::CodexResponses
    );
    if response
        && family != ApiFamily::CodexResponses
        && !string(input.context, "systemPrompt").is_empty()
    {
        out.push(json!({"role":if flag(input.model,"reasoning"){"developer"}else{"system"},"content":input.context["systemPrompt"]}));
    }
    for (index, m) in messages.iter().enumerate() {
        let parts = blocks(&m["content"]);
        match string(m, "role") {
            "user" => {
                if family == ApiFamily::Anthropic && m["content"].is_string() {
                    if !string(m, "content").trim().is_empty() {
                        out.push(json!({"role":"user","content":m["content"]}));
                    }
                    continue;
                }
                let parts = user_parts(&m["content"], family)?;
                if parts.is_empty() {
                    continue;
                }
                out.push(
                    if matches!(
                        family,
                        ApiFamily::Google | ApiFamily::GeminiCli | ApiFamily::Vertex
                    ) {
                        json!({"role":"user","parts":parts})
                    } else {
                        json!({"role":"user","content":parts})
                    },
                );
            }
            "assistant" => {
                let mut converted = vec![];
                for b in parts {
                    let kind = string(&b, "type");
                    let thinking = kind == "thinking";
                    let tool = kind == "toolCall";
                    let value = if response {
                        if thinking {
                            if string(&b, "thinkingSignature").is_empty() {
                                continue;
                            }
                            let v: Value = serde_json::from_str(string(&b, "thinkingSignature"))
                                .map_err(|_| invalid("Responses reasoning signature"))?;
                            if !v.is_object() || v["type"] != "reasoning" {
                                return Err(invalid("Responses reasoning item"));
                            }
                            v
                        } else if tool {
                            let mut ids = string(&b, "id").split('|');
                            let call = ids.next().unwrap_or("");
                            let item = ids.next();
                            let mut v = json!({"type":"function_call","call_id":call,"name":b["name"],"arguments":serde_json::to_string(&b["arguments"]).map_err(|_|invalid("tool arguments"))?});
                            let different_model = m["model"] != input.model["id"]
                                && m["provider"] == input.model["provider"]
                                && m["api"] == input.model["api"];
                            if let Some(item) = item
                                && !(different_model && item.starts_with("fc_"))
                            {
                                v["id"] = json!(item);
                            }
                            v
                        } else {
                            let signature = string(&b, "textSignature");
                            let parsed: Value =
                                serde_json::from_str(signature).unwrap_or(Value::Null);
                            let id = if parsed["v"] == 1 {
                                string(&parsed, "id")
                            } else {
                                signature
                            };
                            let id = if id.is_empty() {
                                format!("msg_{index}")
                            } else if id.len() > 64 {
                                format!("msg_{}", short_hash(id))
                            } else {
                                id.into()
                            };
                            let mut v = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":b["text"],"annotations":[]}],"status":"completed","id":id});
                            if let Some(phase) = parsed.get("phase") {
                                v["phase"] = phase.clone();
                            }
                            v
                        }
                    } else if family == ApiFamily::Anthropic {
                        if tool {
                            json!({"type":"tool_use","id":b["id"],"name":claude_name(string(&b,"name"),input.anthropic_oauth),"input":b["arguments"]})
                        } else if thinking && flag(&b, "redacted") {
                            json!({"type":"redacted_thinking","data":b["thinkingSignature"]})
                        } else {
                            let text = string(&b, if thinking { "thinking" } else { "text" });
                            if text.trim().is_empty() {
                                continue;
                            }
                            if thinking && !string(&b, "thinkingSignature").trim().is_empty() {
                                json!({"type":"thinking","thinking":text,"signature":b["thinkingSignature"]})
                            } else {
                                json!({"type":"text","text":text})
                            }
                        }
                    } else if family == ApiFamily::Bedrock {
                        if tool {
                            json!({"toolUse":{"toolUseId":b["id"],"name":b["name"],"input":b["arguments"]}})
                        } else {
                            let text = string(&b, if thinking { "thinking" } else { "text" });
                            if text.trim().is_empty() {
                                continue;
                            }
                            let supports_sig = string(input.model, "id")
                                .contains("anthropic.claude")
                                || string(input.model, "id").contains("anthropic/claude");
                            if thinking
                                && (!supports_sig
                                    || !string(&b, "thinkingSignature").trim().is_empty())
                            {
                                let mut v =
                                    json!({"reasoningContent":{"reasoningText":{"text":text}}});
                                if supports_sig {
                                    v["reasoningContent"]["reasoningText"]["signature"] =
                                        b["thinkingSignature"].clone();
                                }
                                v
                            } else {
                                json!({"text":text})
                            }
                        }
                    } else {
                        let mut v = if tool {
                            let mut v =
                                json!({"functionCall":{"name":b["name"],"args":b["arguments"]}});
                            if google_ids(input.model) {
                                v["functionCall"]["id"] = b["id"].clone();
                            }
                            v
                        } else {
                            let text = string(&b, if thinking { "thinking" } else { "text" });
                            if text.trim().is_empty() {
                                continue;
                            }
                            let mut v = json!({"text":text});
                            if thinking {
                                v["thought"] = json!(true);
                            }
                            v
                        };
                        let sig = string(
                            &b,
                            if tool {
                                "thoughtSignature"
                            } else if thinking {
                                "thinkingSignature"
                            } else {
                                "textSignature"
                            },
                        );
                        if same(m, input.model) && valid_signature(sig) {
                            v["thoughtSignature"] = json!(sig);
                        } else if tool
                            && string(input.model, "id")
                                .to_lowercase()
                                .contains("gemini-3")
                        {
                            v["thoughtSignature"] = json!("skip_thought_signature_validator");
                        }
                        v
                    };
                    converted.push(value);
                }
                if response {
                    out.extend(converted);
                } else if !converted.is_empty() {
                    out.push(
                        if matches!(
                            family,
                            ApiFamily::Google | ApiFamily::GeminiCli | ApiFamily::Vertex
                        ) {
                            json!({"role":"model","parts":converted})
                        } else {
                            json!({"role":"assistant","content":converted})
                        },
                    );
                }
            }
            "toolResult" => {
                if response {
                    let txt = text(&m["content"]);
                    let has_images = parts.iter().any(|b| b["type"] == "image");
                    let output = if has_images {
                        let mut result = vec![];
                        if !txt.is_empty() {
                            result.push(json!({"type":"input_text","text":txt}));
                        }
                        result.extend(user_parts(
                            &json!(
                                parts
                                    .iter()
                                    .filter(|b| b["type"] == "image")
                                    .collect::<Vec<_>>()
                            ),
                            family,
                        )?);
                        json!(result)
                    } else {
                        json!(if txt.is_empty() {
                            "(see attached image)"
                        } else {
                            &txt
                        })
                    };
                    out.push(json!({"type":"function_call_output","call_id":string(m,"toolCallId").split('|').next().unwrap_or(""),"output":output}));
                } else if family == ApiFamily::Anthropic {
                    append_group(
                        &mut out,
                        json!({"type":"tool_result","tool_use_id":m["toolCallId"],"content":if parts.iter().any(|b|b["type"]=="image"){json!(user_parts(&m["content"],family)?)}else{json!(text(&m["content"]))},"is_error":flag(m,"isError")}),
                        "content",
                        "tool_result",
                    );
                } else if family == ApiFamily::Bedrock {
                    append_group(
                        &mut out,
                        json!({"toolResult":{"toolUseId":m["toolCallId"],"content":user_parts(&m["content"],family)?,"status":if flag(m,"isError"){"error"}else{"success"}}}),
                        "content",
                        "toolResult",
                    );
                } else {
                    let txt = text(&m["content"]);
                    let images: Vec<_> = parts
                        .iter()
                        .filter(|b| b["type"] == "image")
                        .cloned()
                        .collect();
                    let image_parts = user_parts(&json!(images), family)?;
                    let output = if txt.is_empty() && !images.is_empty() {
                        "(see attached image)"
                    } else {
                        &txt
                    };
                    let mut payload = json!({"name":m["toolName"],"response":{}});
                    payload["response"][if flag(m, "isError") {
                        "error"
                    } else {
                        "output"
                    }] = json!(output);
                    if google_ids(input.model) {
                        payload["id"] = m["toolCallId"].clone();
                    }
                    if !images.is_empty() && google_multimodal(input.model) {
                        payload["parts"] = json!(image_parts);
                    }
                    append_group(
                        &mut out,
                        json!({"functionResponse":payload}),
                        "parts",
                        "functionResponse",
                    );
                    if !images.is_empty() && !google_multimodal(input.model) {
                        let mut p = vec![json!({"text":"Tool result image:"})];
                        p.extend(image_parts);
                        out.push(json!({"role":"user","parts":p}));
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    if family == ApiFamily::Anthropic
        && let Some(cache) = cache(input)
        && let Some(last) = out.last_mut()
        && last["role"] == "user"
    {
        if last["content"].is_string() {
            last["content"] = json!([{"type":"text","text":last["content"],"cache_control":cache}]);
        } else if let Some(block) = last["content"].as_array_mut().and_then(|a| a.last_mut()) {
            block["cache_control"] = cache;
        }
    }
    Ok(out)
}
pub(super) fn build(
    input: &RequestContext<'_>,
    family: ApiFamily,
    messages: &[Value],
) -> Result<Value, DomainError> {
    let opts = input.options;
    let model = input.model;
    let defs = tools(input)?;
    let converted = convert(input, family, messages)?;
    let system = string(input.context, "systemPrompt");
    let mut out = match family {
        ApiFamily::Anthropic => {
            let mut v = json!({"model":model["id"],"messages":converted,"stream":true,"max_tokens":opts.get("maxTokens").unwrap_or(&model["maxTokens"])});
            let mut systems = vec![];
            if input.anthropic_oauth {
                systems.push("You are Claude Code, Anthropic's official CLI for Claude.");
            }
            if !system.is_empty() {
                systems.push(system);
            }
            if !systems.is_empty() {
                v["system"] = json!(
                    systems
                        .iter()
                        .map(|s| {
                            let mut p = json!({"type":"text","text":s});
                            if let Some(c) = cache(input) {
                                p["cache_control"] = c;
                            }
                            p
                        })
                        .collect::<Vec<_>>()
                );
            }
            if let Some(defs) = defs {
                v["tools"]=json!(defs.iter().map(|t|json!({"name":claude_name(string(t,"name"),input.anthropic_oauth),"description":t["description"],"input_schema":{"type":"object","properties":t["parameters"].get("properties").cloned().unwrap_or(json!({})),"required":t["parameters"].get("required").cloned().unwrap_or(json!([]))}})).collect::<Vec<_>>());
            }
            if !flag(opts, "thinkingEnabled") {
                option(&mut v, opts, "temperature", "temperature");
            }
            if flag(model, "reasoning") {
                if flag(opts, "thinkingEnabled") {
                    let adaptive = string(model, "id").contains("opus-4-6")
                        || string(model, "id").contains("opus-4.6")
                        || string(model, "id").contains("sonnet-4-6")
                        || string(model, "id").contains("sonnet-4.6");
                    v["thinking"] = if adaptive {
                        json!({"type":"adaptive"})
                    } else {
                        json!({"type":"enabled","budget_tokens":opts.get("thinkingBudgetTokens").unwrap_or(&json!(1024))})
                    };
                    if adaptive && let Some(effort) = opts.get("effort") {
                        v["output_config"] = json!({"effort":effort});
                    }
                } else if opts["thinkingEnabled"] == false {
                    v["thinking"] = json!({"type":"disabled"});
                }
            }
            if let Some(choice) = opts.get("toolChoice") {
                v["tool_choice"] = if choice.is_string() {
                    json!({"type":choice})
                } else {
                    choice.clone()
                };
            }
            if opts["metadata"]["user_id"].is_string() {
                v["metadata"] = json!({"user_id":opts["metadata"]["user_id"]});
            }
            v
        }
        ApiFamily::Responses | ApiFamily::AzureResponses | ApiFamily::CodexResponses => {
            let codex = family == ApiFamily::CodexResponses;
            let mut v = json!({"model":model["id"],"input":converted,"stream":true});
            if family != ApiFamily::AzureResponses {
                v["store"] = json!(false);
            }
            if codex {
                v["instructions"] = json!(system);
                v["text"] =
                    json!({"verbosity":opts.get("textVerbosity").unwrap_or(&json!("medium"))});
                v["include"] = json!(["reasoning.encrypted_content"]);
                v["tool_choice"] = json!("auto");
                v["parallel_tool_calls"] = json!(true);
            } else {
                option(&mut v, opts, "maxTokens", "max_output_tokens");
            }
            option(&mut v, opts, "temperature", "temperature");
            option(&mut v, opts, "toolChoice", "tool_choice");
            option(&mut v, opts, "sessionId", "prompt_cache_key");
            if let Some(defs) = defs {
                v["tools"]=json!(defs.iter().map(|t|json!({"type":"function","name":t["name"],"description":t["description"],"parameters":t["parameters"],"strict":if codex{Value::Null}else{json!(false)}})).collect::<Vec<_>>());
            }
            let effort = string(opts, "reasoningEffort");
            if !effort.is_empty() {
                v["reasoning"] = json!({"effort":effort,"summary":opts.get("reasoningSummary").unwrap_or(&json!("auto"))});
                v["include"] = json!(["reasoning.encrypted_content"]);
            } else if flag(model, "reasoning") && !codex {
                v["reasoning"] = json!({"effort":"none"});
            }
            v
        }
        ApiFamily::Google | ApiFamily::GeminiCli | ApiFamily::Vertex => {
            let mut v = json!({"contents":converted,"generationConfig":{}});
            option(
                &mut v["generationConfig"],
                opts,
                "maxTokens",
                "maxOutputTokens",
            );
            option(
                &mut v["generationConfig"],
                opts,
                "temperature",
                "temperature",
            );
            if !system.is_empty() {
                v["systemInstruction"] = json!({"parts":[{"text":system}]});
                if family != ApiFamily::GeminiCli {
                    v["systemInstruction"]["role"] = json!("user");
                }
            }
            if let Some(defs) = defs
                && !defs.is_empty()
            {
                let use_parameters =
                    family == ApiFamily::GeminiCli && string(model, "id").starts_with("claude-");
                v["tools"] = json!([{"functionDeclarations":defs.iter().map(|t|{let mut p=json!({"name":t["name"],"description":t["description"]});p[if use_parameters{"parameters"}else{"parametersJsonSchema"}]=t["parameters"].clone();p}).collect::<Vec<_>>()}]);
            }
            if let Some(choice) = opts["toolChoice"].as_str() {
                v["toolConfig"] = json!({"functionCallingConfig":{"mode":match choice{"none"=>"NONE","any"=>"ANY",_=>"AUTO"}}});
            }
            option(
                &mut v["generationConfig"],
                opts,
                "thinking",
                "thinkingConfig",
            );
            if family == ApiFamily::GeminiCli {
                let project = input
                    .google_project
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| invalid("Google project identity"))?;
                if input.request_id.is_empty() {
                    return Err(invalid("Google request identity"));
                }
                json!({"project":project,"model":model["id"],"request":v,"userAgent":"ag1-coding-agent","requestId":input.request_id})
            } else {
                v
            }
        }
        ApiFamily::Bedrock => {
            // HTTP JSON shape: modelId belongs to the URL and bytes are base64 strings.
            let mut v = json!({"messages":converted,"inferenceConfig":{}});
            if !system.is_empty() {
                v["system"] = json!([{"text":system}]);
            }
            option(&mut v["inferenceConfig"], opts, "maxTokens", "maxTokens");
            option(
                &mut v["inferenceConfig"],
                opts,
                "temperature",
                "temperature",
            );
            if let Some(defs) = defs
                && !defs.is_empty()
            {
                v["toolConfig"] = json!({"tools":defs.iter().map(|t|json!({"toolSpec":{"name":t["name"],"description":t["description"],"inputSchema":{"json":t["parameters"]}}})).collect::<Vec<_>>()});
                if let Some(choice) = opts.get("toolChoice") {
                    v["toolConfig"]["toolChoice"] = if let Some(name) = choice.as_str() {
                        if ["auto", "any"].contains(&name) {
                            let mut c = json!({});
                            c[name] = json!({});
                            c
                        } else {
                            json!({"tool":{"name":name}})
                        }
                    } else {
                        choice.clone()
                    };
                }
            }
            option(&mut v, opts, "requestMetadata", "requestMetadata");
            option(
                &mut v,
                opts,
                "additionalModelRequestFields",
                "additionalModelRequestFields",
            );
            v
        }
        _ => unreachable!(),
    };
    // Empty generation configs are valid and preserve explicit provider defaults.
    if out.is_null() {
        return Err(invalid("provider payload"));
    }
    Ok(out.take())
}
