use super::*;

#[test]
fn native_health_matches_twenty_seven_unchanged_sdk_analyses() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/health-oracle.json"
    ))
    .unwrap();
    for row in oracle["rows"].as_array().unwrap() {
        let actual = ag1_core::runtime::health::analyze(
            &row["message"],
            row["volumeRails"].as_bool().unwrap(),
        );
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            row["analysis"],
            "{}",
            row["id"]
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_health_reports_small_context_volume_without_shortening_output() {
    for context in [32768, 200000] {
        let text = "x".repeat(48000);
        let reply = json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":14000}});
        let (origin, server) = server(vec![reply]);
        let fixture = fixture(&origin, "disabled");
        let mut model = fixture.model.clone();
        model["contextWindow"] = json!(context);
        fixture
            .host
            .select_model("health-model", "health-model", NOW, model, &ignore)
            .await
            .unwrap();
        let events = Mutex::new(Vec::new());
        let listener = |event: &SessionEvent| {
            if let SessionEvent::Recovery(value) = event {
                events.lock().unwrap().push(value.clone());
            }
        };
        let result = fixture
            .host
            .run(
                input(),
                &Resolver::new(false),
                fixture.files.clone(),
                &listener,
            )
            .await
            .unwrap();
        assert_eq!(result.assistant["content"][0]["text"], text);
        let events = events.lock().unwrap();
        let degraded = events
            .iter()
            .filter(|v| v["type"] == "degraded_turn")
            .collect::<Vec<_>>();
        assert_eq!(degraded.len(), usize::from(context == 32768));
        if let Some(event) = degraded.first() {
            assert_eq!(event["severity"], "critical");
            assert_eq!(
                event["reasons"],
                json!(["very_high_output_tokens", "very_long_output_text"])
            );
        }
        assert_eq!(result.generations, 1);
        assert!(fixture.host.settled().unwrap());
        assert_eq!(server.join().unwrap().len(), 1);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_health_does_not_treat_large_tool_arguments_as_verbose_prose() {
    let content = "x".repeat(56000);
    let mut write = calls(vec![(
        "large-write",
        "write",
        json!({"path":"large.txt","content":content}),
    )]);
    write["usage"] = json!({"prompt_tokens":100,"completion_tokens":14000});
    let (origin, server) = server(vec![write, stop()]);
    let fixture = fixture(&origin, "disabled");
    let events = Mutex::new(Vec::new());
    let listener = |event: &SessionEvent| {
        if let SessionEvent::Recovery(value) = event {
            events.lock().unwrap().push(value.clone());
        }
    };
    let result = fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &listener,
        )
        .await
        .unwrap();
    assert_eq!(result.generations, 2);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("large.txt")).unwrap(),
        content
    );
    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|v| v["type"] == "degraded_turn")
    );
    assert_eq!(server.join().unwrap().len(), 2);
}
