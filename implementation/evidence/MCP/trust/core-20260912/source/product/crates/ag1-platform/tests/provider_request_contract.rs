use ag1_platform::provider::requests::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

#[test]
fn text_only_models_can_continue_after_tool_images_without_losing_journal_images() {
    use ag1_platform::provider::requests::{RequestContext, build_payload};
    let context = json!({"messages":[
        {"role":"user","content":"Make an image"},
        {"role":"assistant","provider":"fixture","model":"fixture","api":"openai-completions","content":[{"type":"toolCall","id":"preview-call","name":"read","arguments":{"path":"ball.svg"}}],"stopReason":"toolUse"},
        {"role":"toolResult","toolCallId":"preview-call","toolName":"read","isError":false,"content":[{"type":"text","text":"SVG source retained"},{"type":"image","mimeType":"image/png","data":"aW1hZ2UtYnl0ZXM="}]}
    ]});
    let original = context.clone();
    for vision in [false, true] {
        let model = json!({"id":"fixture","provider":"fixture","api":"openai-completions","input":if vision {json!(["text","image"])} else {json!(["text"])},"maxTokens":1024});
        let result = build_payload(&RequestContext { model: &model, context: &context, options: &json!({}), anthropic_oauth: false, google_project: None, request_id: "image-preview" }).unwrap();
        let encoded = result.to_string();
        assert!(encoded.contains("SVG source retained"));
        assert_eq!(encoded.contains("has not visually inspected"), !vision);
        assert_eq!(encoded.contains("aW1hZ2UtYnl0ZXM="), vision);
        assert_eq!(context, original, "provider projection must not mutate journal content");
    }
    let model = json!({"id":"fixture","provider":"fixture","api":"openai-completions","input":["text"]});
    let user_image = json!({"messages":[{"role":"user","content":[{"type":"image","mimeType":"image/png","data":"aW1hZ2U="}]}]});
    assert!(build_payload(&RequestContext { model: &model, context: &user_image, options: &json!({}), anthropic_oauth: false, google_project: None, request_id: "user-image" }).is_err(), "user image input still requires vision support");
}
fn transport_body(mut v: Value, api: &str) -> Value {
    // Original Bedrock capture is the SDK command. HTTP serialization places modelId
    // in the URL and encodes Uint8Array image bytes as base64, independently checked below.
    if api == "bedrock-converse-stream" {
        v.as_object_mut().unwrap().remove("modelId");
        fn encode(v: &mut Value) {
            match v {
                Value::Object(o) => {
                    if let Some(bytes) = o.get("bytes").and_then(Value::as_object) {
                        let mut pairs: Vec<_> = bytes
                            .iter()
                            .map(|(k, v)| (k.parse::<usize>().unwrap(), v.as_u64().unwrap() as u8))
                            .collect();
                        pairs.sort_by_key(|p| p.0);
                        o.insert(
                            "bytes".into(),
                            json!(STANDARD.encode(pairs.iter().map(|p| p.1).collect::<Vec<_>>())),
                        );
                    }
                    for value in o.values_mut() {
                        encode(value);
                    }
                }
                Value::Array(a) => {
                    for value in a {
                        encode(value);
                    }
                }
                _ => {}
            }
        }
        encode(&mut v);
    }
    v
}
fn base_context() -> Value {
    json!({"systemPrompt":"Fixture system","messages":[{"role":"user","content":"Return a tool call or answer.","timestamp":0}],"tools":[{"name":"read","description":"Fixture read","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]})
}
#[test]
fn all_ten_base_requests_match_original_sdk() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/providers/oracle.json")).unwrap();
    for group in oracle["groups"].as_array().unwrap() {
        let api = group["api"].as_str().unwrap();
        let model = json!({"id":"fixture","api":api,"provider":if api=="openai-codex-responses"{"openai-codex"}else{api},"baseUrl":"https://fixture.invalid/v1","reasoning":true,"input":["text","image"],"maxTokens":4096});
        let got = build_payload(&RequestContext {
            model: &model,
            context: &base_context(),
            options: &json!({"maxTokens":256}),
            anthropic_oauth: false,
            google_project: Some("fixture-project"),
            request_id: "<fixture-request-id>",
        })
        .unwrap();
        assert_eq!(
            got,
            transport_body(group["transport"][0]["body"].clone(), api),
            "{api}"
        );
    }
}
#[test]
fn tool_roundtrips_model_switches_and_missing_results_match_original_sdk() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/providers/requests.json")).unwrap();
    let mut failed = vec![];
    for group in oracle["groups"].as_array().unwrap() {
        let api = group["api"].as_str().unwrap();
        let got = build_payload(&RequestContext {
            model: &group["model"],
            context: &group["context"],
            options: &group["options"],
            anthropic_oauth: false,
            google_project: Some("fixture-project"),
            request_id: "<fixture-request-id>",
        })
        .unwrap();
        let want = transport_body(group["transport"][0]["body"].clone(), api);
        if got != want {
            eprintln!("{} {}\ngot {}\nwant {}", api, group["scenario"], got, want);
            failed.push(format!("{api}/{}", group["scenario"]));
        }
    }
    assert!(failed.is_empty(), "mismatches: {failed:?}");
}
#[test]
fn all_six_preserved_llama_request_payloads_match_p01() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/llama/request-payloads.json"
    ))
    .unwrap();
    for group in oracle["payloads"].as_array().unwrap() {
        let model = json!({"id":group["model"],"api":"openai-completions","provider":"llamacpp","reasoning":true,"input":["text","image"],"compat":{"thinkingFormat":"qwen-chat-template"}});
        let mut options = group["input"].clone();
        if group["reasoning"] != "off" {
            options["reasoningEffort"] = group["reasoning"].clone();
        }
        let got = build_payload(&RequestContext {
            model: &model,
            context: &base_context(),
            options: &options,
            anthropic_oauth: false,
            google_project: None,
            request_id: "fixture",
        })
        .unwrap();
        assert_eq!(
            got, group["payload"],
            "{} {}",
            group["model"], group["reasoning"]
        );
    }
}

#[test]
fn thinking_cache_sampling_and_tool_choice_match_original_sdk() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/providers/options.json")).unwrap();
    let mut failed = vec![];
    for group in oracle["groups"].as_array().unwrap() {
        let api = group["api"].as_str().unwrap();
        let mut options = group["options"].clone();
        // The reference transport is explicitly configured with this Azure deployment.
        if api == "azure-openai-responses" {
            options["azureDeploymentName"] = json!("fixture");
        }
        let got = build_payload(&RequestContext {
            model: &group["model"],
            context: &group["context"],
            options: &options,
            anthropic_oauth: false,
            google_project: Some("fixture-project"),
            request_id: "<fixture-request-id>",
        })
        .unwrap();
        let want = transport_body(group["transport"][0]["body"].clone(), api);
        if got != want {
            eprintln!("{} {}\ngot {}\nwant {}", api, group["scenario"], got, want);
            failed.push(format!("{api}/{}", group["scenario"]));
        }
    }
    assert!(failed.is_empty(), "mismatches: {failed:?}");
}

#[test]
fn bedrock_http_payloads_match_actual_sdk_serialization() {
    let requests: Value =
        serde_json::from_str(include_str!("../../../../fixtures/providers/requests.json")).unwrap();
    let aws: Value =
        serde_json::from_str(include_str!("../../../../fixtures/providers/aws.json")).unwrap();
    for group in requests["groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["api"] == "bedrock-converse-stream")
    {
        let expected = aws["serialized"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["scenario"] == group["scenario"])
            .unwrap();
        let payload = build_payload(&RequestContext {
            model: &group["model"],
            context: &group["context"],
            options: &group["options"],
            anthropic_oauth: false,
            google_project: None,
            request_id: "fixture",
        })
        .unwrap();
        assert_eq!(payload, expected["body"]);
    }
}

#[test]
fn native_template_effort_keeps_casing_and_can_disable_via_none() {
    let model = json!({"id":"native","api":"openai-completions","provider":"llamacpp","reasoning":true,"input":["text"],"maxTokens":4096,"compat":{"thinkingFormat":"qwen-chat-template"},"thinking":{"apply":"reasoning_effort","modes":[{"id":"off","nativeValue":"none"},{"id":"xhigh","nativeValue":"xHigh"},{"id":"max","nativeValue":"max"}]}});
    for (level, native) in [("xhigh", "xHigh"), ("off", "none"), ("max", "max")] {
        let options =
            ag1_platform::provider::requests::session_options(&model, level, None).unwrap();
        let payload = build_payload(&RequestContext {
            model: &model,
            context: &base_context(),
            options: &options,
            anthropic_oauth: false,
            google_project: None,
            request_id: "native",
        })
        .unwrap();
        assert_eq!(payload["chat_template_kwargs"]["reasoning_effort"], native);
        assert_eq!(
            payload["chat_template_kwargs"]["enable_thinking"],
            level != "off"
        );
    }
}
