use ag1_core::{DomainError, provider::*};
use ag1_platform::provider::framing::{AwsEventDecoder, event_crc32};
use ag1_platform::provider::{WireDecoder, framing::SseDecoder};
use serde_json::Value;
fn fixtures() -> (Value, Value) {
    (
        serde_json::from_str(include_str!("../../../../fixtures/providers/wire.json")).unwrap(),
        serde_json::from_str(include_str!("../../../../fixtures/providers/oracle.json")).unwrap(),
    )
}
fn apply(decoder: &mut WireDecoder, reducer: &mut StreamReducer, value: &Value) {
    for event in decoder.decode(value).unwrap() {
        reducer.apply(event, 1).unwrap();
    }
}
#[test]
fn ten_provider_families_match_pinned_sdk_content_tools_signatures_usage_and_cost() {
    let (wire, oracle) = fixtures();
    assert_eq!(
        wire["cases"].as_array().unwrap().len(),
        BUILTIN_FAMILIES.len()
    );
    for (case, expected) in wire["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["groups"].as_array().unwrap())
    {
        let family: ApiFamily = serde_json::from_value(case["api"].clone()).unwrap();
        assert_eq!(case["api"], expected["api"]);
        for chunk_size in [1, 7, 31, 4096] {
            let mut decoder = WireDecoder::new(family);
            let mut reducer = StreamReducer::new(family, 0, 1024 * 1024).unwrap();
            if family == ApiFamily::Bedrock {
                // SDK-decoded AWS events. Binary event-stream framing is tested separately.
                for value in case["frames"].as_array().unwrap() {
                    apply(&mut decoder, &mut reducer, value);
                }
            } else {
                let bytes = case["frames"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| format!("data: {v}\r\n\r\n"))
                    .collect::<String>()
                    .into_bytes();
                let mut sse = SseDecoder::new(1024 * 1024).unwrap();
                for chunk in bytes.chunks(chunk_size) {
                    for frame in sse.feed(chunk).unwrap() {
                        for event in decoder.decode_json(frame.data.as_bytes()).unwrap() {
                            reducer.apply(event, 1).unwrap();
                        }
                    }
                }
                sse.finish().unwrap();
                assert_eq!(sse.retained_bytes(), 0);
            }
            let result = reducer.finish(false).unwrap();
            let reference = &expected["result"];
            assert_eq!(result.stop_reason, Some(StopReason::ToolUse), "{family:?}");
            assert_eq!(
                result.response_id.as_deref(),
                reference["responseId"].as_str(),
                "{family:?}"
            );
            assert_eq!(
                result.content.len(),
                reference["content"].as_array().unwrap().len(),
                "{family:?}"
            );
            for (block, original) in result
                .content
                .iter()
                .zip(reference["content"].as_array().unwrap())
            {
                match block.kind {
                    BlockKind::Text => {
                        assert_eq!(original["type"], "text");
                        assert_eq!(block.text, original["text"], "{family:?}");
                    }
                    BlockKind::Thinking => {
                        assert_eq!(original["type"], "thinking");
                        assert_eq!(block.text, original["thinking"], "{family:?}");
                    }
                    BlockKind::Tool => {
                        assert_eq!(block.id, original["id"], "{family:?}");
                        assert_eq!(block.name, original["name"], "{family:?}");
                        assert_eq!(
                            block.arguments.as_ref().unwrap(),
                            &original["arguments"],
                            "{family:?}"
                        );
                        assert!(!block.malformed_arguments);
                    }
                }
                let signature = original
                    .get("thinkingSignature")
                    .or_else(|| original.get("textSignature"))
                    .or_else(|| original.get("thoughtSignature"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if signature.starts_with('{') {
                    // JSON replay envelopes are semantic values; encrypted signatures remain exact bytes.
                    assert_eq!(
                        serde_json::from_str::<Value>(&block.signature).unwrap(),
                        serde_json::from_str::<Value>(signature).unwrap(),
                        "{family:?}"
                    );
                } else {
                    assert_eq!(block.signature, signature, "{family:?}");
                }
            }
            let usage = &reference["usage"];
            for (actual, key) in [
                (result.usage.input, "input"),
                (result.usage.output, "output"),
                (result.usage.cache_read, "cacheRead"),
                (result.usage.cache_write, "cacheWrite"),
                (result.usage.total, "totalTokens"),
            ] {
                assert_eq!(actual, usage[key].as_u64().unwrap(), "{family:?}/{key}");
            }
            let cost = result.cost([1.0, 2.0, 0.1, 1.25]).unwrap();
            for (index, key) in ["input", "output", "cacheRead", "cacheWrite", "total"]
                .iter()
                .enumerate()
            {
                assert!(
                    (cost[index] - usage["cost"][key].as_f64().unwrap()).abs() < 1e-15,
                    "{family:?}/{key}"
                );
            }
        }
    }
}
#[test]
fn each_family_errors_before_content_mid_stream_and_after_stop_remain_failures() {
    let (wire, _) = fixtures();
    for case in wire["cases"].as_array().unwrap() {
        let family: ApiFamily = serde_json::from_value(case["api"].clone()).unwrap();
        let frames = case["frames"].as_array().unwrap();
        for count in [0, frames.len() / 2, frames.len()] {
            let mut decoder = WireDecoder::new(family);
            let mut reducer = StreamReducer::new(family, 0, 1024 * 1024).unwrap();
            for value in &frames[..count] {
                apply(&mut decoder, &mut reducer, value);
            }
            let before = reducer.snapshot();
            apply(&mut decoder, &mut reducer, &case["error"]);
            let result = reducer.finish(false).unwrap();
            assert_eq!(result.stop_reason, Some(StopReason::Error), "{family:?}");
            assert!(
                result.error.unwrap().contains("fixture provider failure"),
                "{family:?}"
            );
            assert_eq!(before.content.len(), result.content.len());
            assert!(matches!(reducer.finish(false), Err(DomainError::Conflict)));
        }
    }
}
#[test]
fn sse_handles_multiline_utf8_cr_lf_comments_and_truncated_frames() {
    for ending in ["\n", "\r", "\r\n"] {
        let input = format!(
            ":heartbeat{ending}event: message{ending}data: {{\"text\":{ending}data: \"🌙\"}}{ending}{ending}"
        );
        for size in 1..=input.len() {
            let mut decoder = SseDecoder::new(1024).unwrap();
            let mut messages = vec![];
            for chunk in input.as_bytes().chunks(size) {
                messages.extend(decoder.feed(chunk).unwrap());
            }
            decoder.finish().unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].event, "message");
            assert_eq!(
                serde_json::from_str::<Value>(&messages[0].data).unwrap()["text"],
                "🌙"
            );
        }
    }
    let mut decoder = SseDecoder::new(1024).unwrap();
    decoder.feed(b"data: unfinished").unwrap();
    assert!(decoder.finish().is_err());
    assert!(decoder.feed(&vec![b'a'; 1024]).is_err());
    let mut decoder = SseDecoder::new(1024).unwrap();
    assert!(decoder.feed(b"data: \xff\n\n").is_err());
}
#[test]
fn aws_binary_frames_match_sdk_encoder_and_reject_every_checksum_mutation() {
    let bytes = include_bytes!("../../../../fixtures/providers/bedrock-events.bin");
    let (wire, _) = fixtures();
    let expected = &wire["cases"].as_array().unwrap().last().unwrap()["frames"];
    for size in [1, 2, 7, 31, 8192] {
        let mut decoder = AwsEventDecoder::new(1024 * 1024).unwrap();
        let mut actual = vec![];
        for part in bytes.chunks(size) {
            for message in decoder.feed(part).unwrap() {
                actual.push(message.json_event().unwrap());
            }
        }
        decoder.finish().unwrap();
        assert_eq!(serde_json::json!(actual), *expected);
    }
    assert_eq!(event_crc32(b"123456789"), 0xcbf43926);
    for index in 0..bytes.len() {
        let mut corrupted = bytes.to_vec();
        corrupted[index] ^= 1;
        let mut decoder = AwsEventDecoder::new(1024 * 1024).unwrap();
        assert!(
            decoder.feed(&corrupted).is_err() || decoder.finish().is_err(),
            "byte {index}"
        );
    }
    for end in 1..12 {
        let mut decoder = AwsEventDecoder::new(1024).unwrap();
        decoder.feed(&bytes[..end]).unwrap();
        assert!(decoder.finish().is_err());
    }
}

#[test]
fn missing_google_tool_ids_are_scoped_to_durable_request_identity() {
    let payload = serde_json::json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"path":"a"}}}]}}]});
    let ids: Vec<_> = ["request-one", "request-two"]
        .iter()
        .map(|id| {
            let mut decoder = WireDecoder::for_request(ApiFamily::Google, id).unwrap();
            decoder
                .decode(&payload)
                .unwrap()
                .into_iter()
                .find_map(|e| {
                    if let ProviderEvent::Begin {
                        kind: BlockKind::Tool,
                        id,
                        ..
                    } = e
                    {
                        Some(id)
                    } else {
                        None
                    }
                })
                .unwrap()
        })
        .collect();
    assert_ne!(ids[0], ids[1]);
    assert!(
        WireDecoder::new(ApiFamily::Google)
            .decode(&payload)
            .is_err()
    );
}

#[test]
fn anthropic_oauth_tool_names_map_back_to_declared_tool_identity() {
    let mut decoder = WireDecoder::for_request(ApiFamily::Anthropic, "oauth-turn")
        .unwrap()
        .with_tool_names(&["read".into(), "webSearch".into()])
        .unwrap();
    let events=decoder.decode(&serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool-1","name":"Read","input":{}}})).unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e,ProviderEvent::Begin{name,..}if name=="read"))
    );
    assert!(
        WireDecoder::new(ApiFamily::Anthropic)
            .with_tool_names(&["read".into(), "READ".into()])
            .is_err()
    );
}

#[test]
fn response_service_tier_pricing_tracks_server_override() {
    let mut reducer = StreamReducer::new(ApiFamily::Responses, 0, 4096).unwrap();
    reducer
        .apply(ProviderEvent::ServiceTier("priority".into()), 0)
        .unwrap();
    let mut decoder = WireDecoder::new(ApiFamily::Responses);
    let events=decoder.decode(&serde_json::json!({"type":"response.completed","response":{"status":"completed","service_tier":"flex","usage":{"input_tokens":1000000,"output_tokens":0}}})).unwrap();
    for event in events {
        reducer.apply(event, 1).unwrap();
    }
    let result = reducer.finish(false).unwrap();
    assert_eq!(result.cost([1.0, 2.0, 0.1, 1.25]).unwrap()[0], 0.5);
}

#[test]
fn responses_additive_events_and_reasoning_text_preserve_tool_execution() {
    use serde_json::json;
    for family in [ApiFamily::Responses, ApiFamily::CodexResponses] {
        let mut decoder = WireDecoder::new(family);
        let mut reducer = StreamReducer::new(family, 0, 4096).unwrap();
        for v in [
            json!({"type":"response.created","response":{"id":"resp-test"}}),
            json!({"type":"ping"}),
            json!({"type":"rate_limits.updated","rate_limits":[]}),
            json!({"type":"response.future_metadata","untrusted":"ignored"}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"r","type":"reasoning"}}),
            json!({"type":"response.reasoning_text.delta","item_id":"r","delta":"Checking GitHub"}),
            json!({"type":"response.reasoning_text.done","item_id":"r"}),
            json!({"type":"response.output_item.done","item":{"id":"r","type":"reasoning","summary":[]}}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"f","call_id":"c","type":"function_call","name":"github","arguments":""}}),
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"f","type":"function_call","arguments":"{\"operation\":\"repos.list\"}"}}),
            json!({"type":"response.completed","response":{"status":"completed"}}),
        ] { apply(&mut decoder, &mut reducer, &v); }
        let result = reducer.finish(false).unwrap();
        assert_eq!(result.stop_reason, Some(StopReason::ToolUse));
        assert_eq!(result.content[0].text, "Checking GitHub");
        assert_eq!(result.content[1].name, "github");
        assert_eq!(result.content[1].arguments, Some(json!({"operation":"repos.list"})));
        assert!(decoder.decode(&json!({"type":"response.output_item.added","item":{"type":"unknown_executable"}})).is_err());
        assert!(decoder.decode(&json!({})).is_err());
        for kind in ["response.failed", "response.cancelled", "error"] {
            assert!(matches!(decoder.decode(&json!({"type":kind})).unwrap().as_slice(), [ProviderEvent::Fail(_)]));
        }
    }
}
