use ag1_core::model::VisionVerdict;
use ag1_platform::provider::{capability::*, http::*};
use serde_json::Value;
#[test]
fn forty_two_vision_probes_match_reference_without_inference() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../../fixtures/providers/vision.json")).unwrap();
    for group in fixture["groups"].as_array().unwrap() {
        let kind = match group["kind"].as_str().unwrap() {
            "llamacpp" => ProbeKind::LlamaProps,
            "lmstudio" => ProbeKind::LmStudio,
            "ollama" => ProbeKind::Ollama,
            "canary" => ProbeKind::Canary,
            _ => panic!("unknown fixture kind"),
        };
        let model = group["model"].as_str().unwrap();
        let observation = interpret_probe(
            kind,
            model,
            group["status"].as_u64().unwrap() as u16,
            group["bytes"].as_str().unwrap().as_bytes(),
        );
        let expected: VisionVerdict =
            serde_json::from_value(group["result"]["verdict"].clone()).unwrap();
        assert_eq!(observation.verdict, expected, "{group}");
        assert_eq!(
            observation.source.as_deref(),
            group["result"]["source"].as_str()
        );
        if group.get("request").is_some() {
            let request = probe_request(
                kind,
                "https://fixture.invalid/v1",
                model,
                Some(&ag1_core::SecretBytes::new(b"fixture-key".to_vec())),
            )
            .unwrap();
            assert_eq!(request.url, group["request"]["url"]);
            assert_eq!(
                match request.method {
                    HttpMethod::Get => "GET",
                    HttpMethod::Post => "POST",
                },
                group["request"]["method"]
            );
            let body = if request.body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&request.body).unwrap()
            };
            assert_eq!(body, group["request"]["body"]);
        }
    }
}
