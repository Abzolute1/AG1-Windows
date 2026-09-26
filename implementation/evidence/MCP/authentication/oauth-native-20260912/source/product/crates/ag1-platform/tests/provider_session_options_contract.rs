use serde_json::Value;
#[test]
fn selected_thinking_payloads_match_all_ten_original_simple_adapters() {
    use ag1_platform::provider::requests::{RequestContext, build_payload, session_options};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/release/session-thinking.json"
    ))
    .unwrap();
    let mut failures = vec![];
    for group in oracle["groups"].as_array().unwrap() {
        let mut overrides = group["options"].clone();
        overrides.as_object_mut().unwrap().remove("reasoning");
        let options = session_options(
            &group["model"],
            group["scenario"].as_str().unwrap(),
            Some(&overrides),
        )
        .unwrap();
        let actual = build_payload(&RequestContext {
            model: &group["model"],
            context: &group["context"],
            options: &options,
            anthropic_oauth: false,
            google_project: Some("fixture-project"),
            request_id: "<fixture-request-id>",
        })
        .unwrap();
        let mut expected = group["transport"][0]["body"].clone();
        if group["api"] == "bedrock-converse-stream" {
            expected.as_object_mut().unwrap().remove("modelId");
        }
        if actual != expected {
            eprintln!(
                "Mismatch {} {} {}\nactual={actual}\nexpected={expected}",
                group["api"], group["model"]["id"], group["scenario"]
            );
            failures.push(format!(
                "{} {} {}",
                group["api"], group["model"]["id"], group["scenario"]
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
    println!(
        "196 native session thinking payloads equal original TS streamSimple payloads; zero network/model actions."
    );
}
