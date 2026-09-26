use super::*;

fn response(text: &str) -> Value {
    json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":2}})
}
fn messages(store: &Store) -> Vec<Value> {
    store
        .snapshot("session")
        .unwrap()
        .entries
        .iter()
        .filter_map(|entry| {
            let value: Value = serde_json::from_slice(&entry.normalized).unwrap();
            (value["type"] == "message").then(|| value["message"].clone())
        })
        .collect()
}
fn input_without_compaction() -> TurnInput {
    let mut input = input();
    input.compaction.enabled = false;
    input.retry.enabled = false;
    input
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xml_mutations_must_pass_the_same_approval_policy_as_native_tool_calls() {
    let source = "<write><path>forbidden.txt</path><content>requires approval</content></write>";
    let (origin, server) = server(vec![response(source), stop()]);
    let f = fixture_configured(&origin, "interactive", vec![], None, None, 1);
    f.host
        .run(
            input_without_compaction(),
            &Resolver::new(false),
            f.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert!(!f.root.join("forbidden.txt").exists());
    let journal = messages(&f.store);
    let results: Vec<_> = journal
        .iter()
        .filter(|m| m["role"] == "toolResult")
        .collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["isError"], true);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1]["messages"].to_string().contains("tool"));
    assert!(f.host.settled().unwrap());
    assert!(f.host.persistence_healthy().unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xml_calls_share_native_receipts_keep_original_output_and_continue_from_results() {
    let source = "<tool_call><function=write><parameter=path>xml.txt</parameter><parameter=content>retained XML result</parameter></function></tool_call>\n<read><path>xml.txt</path><offset>1</offset></read>";
    let (origin, server) = server(vec![response(source), stop()]);
    let f = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    let events = Mutex::new(vec![]);
    let listener = |event: &SessionEvent| {
        if let SessionEvent::Recovery(value) = event {
            events.lock().unwrap().push(value.clone());
        }
    };
    let result = f
        .host
        .run(
            input_without_compaction(),
            &Resolver::new(false),
            f.files.clone(),
            &listener,
        )
        .await
        .unwrap();
    assert_eq!(result.generations, 2);
    assert_eq!(
        std::fs::read_to_string(f.root.join("xml.txt")).unwrap(),
        "retained XML result"
    );
    let requests = server.join().unwrap();
    assert_eq!(
        requests[0]["messages"][0], requests[1]["messages"][0],
        "system prompt remains byte-stable"
    );
    assert!(
        requests[1]["messages"]
            .to_string()
            .contains("XML fallback recovery mode")
    );
    assert!(
        requests[1]["messages"]
            .to_string()
            .contains("retained XML result")
    );
    let journal = messages(&f.store);
    let assistant = journal
        .iter()
        .find(|m| m["nativeXmlFallback"].is_object())
        .unwrap();
    assert_eq!(assistant["content"][0]["text"], source);
    assert_eq!(assistant["nativeXmlFallback"]["analysis"]["callCount"], 2);
    let calls = assistant["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["type"] == "toolCall")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    let results = journal
        .iter()
        .filter(|m| m["role"] == "toolResult")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 2);
    for (call, result) in calls.iter().zip(results) {
        assert_eq!(call["id"], result["toolCallId"]);
        assert_eq!(result["isError"], false);
    }
    let raw = provider_outputs(&f.store);
    assert!(
        raw.iter()
            .any(|value| value["snapshot"]["content"][0]["text"] == source)
    );
    assert!(
        !raw.iter()
            .filter(|value| value["kind"] == "runtime-provider-output")
            .any(|value| value.to_string().contains("xml-call-"))
    );
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "xml_tool_call")
            .count(),
        1
    );
    assert!(f.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structured_calls_suppress_xml_execution_in_the_same_response() {
    let mut mixed = calls(vec![(
        "native-only",
        "append",
        json!({"path":"once.txt","content":"native"}),
    )]);
    mixed["choices"][0]["delta"]["content"] =
        json!("<append><path>once.txt</path><content>duplicate XML</content></append>");
    let (origin, server) = server(vec![mixed, stop()]);
    let f = fixture(&origin, "disabled");
    std::fs::write(f.root.join("once.txt"), "").unwrap();
    f.host
        .run(
            input_without_compaction(),
            &Resolver::new(false),
            f.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.join("once.txt")).unwrap(),
        "native"
    );
    assert_eq!(
        messages(&f.store)
            .iter()
            .filter(|m| m["role"] == "toolResult")
            .count(),
        1
    );
    assert!(
        !messages(&f.store)
            .iter()
            .any(|m| m["nativeXmlFallback"].is_object())
    );
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xml_duplicate_turns_and_three_iteration_limit_do_not_repeat_mutations() {
    for repeated in [true, false] {
        let sources: Vec<_> = if repeated {
            vec!["A", "A"]
        } else {
            vec!["A", "B", "C", "D"]
        }
        .into_iter()
        .map(|text| format!("<append><path>once.txt</path><content>{text}</content></append>"))
        .collect();
        let (origin, server) = server(sources.iter().map(|text| response(text)).collect());
        let f = fixture_configured(&origin, "disabled", vec![], None, None, 1);
        std::fs::write(f.root.join("once.txt"), "").unwrap();
        let result = f
            .host
            .run(
                input_without_compaction(),
                &Resolver::new(false),
                f.files.clone(),
                &ignore,
            )
            .await
            .unwrap();
        assert_eq!(result.generations, sources.len() as u64);
        assert_eq!(
            std::fs::read_to_string(f.root.join("once.txt")).unwrap(),
            if repeated { "A" } else { "A\nB\nC" }
        );
        let journal = messages(&f.store);
        let ids: std::collections::BTreeSet<_> = journal
            .iter()
            .filter(|m| m["role"] == "toolResult")
            .map(|m| m["toolCallId"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), if repeated { 1 } else { 3 });
        assert_eq!(server.join().unwrap().len(), sources.len());
        assert!(f.host.settled().unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xml_examples_in_code_inactive_names_and_failed_generations_do_not_execute() {
    let xml = "<write><path>forbidden.txt</path><content>forbidden</content></write>";
    for kind in ["fenced", "inactive", "failed"] {
        let reply = match kind {
            "fenced" => response(&format!("```xml\n{xml}\n```")),
            "inactive" => response(&xml.replace("write>", "unavailable>")),
            _ => {
                json!({"fixtureFrames":[{"choices":[{"delta":{"content":xml}}]},{"error":{"message":"fixture output failed"}}]})
            }
        };
        let (origin, server) = server(vec![reply]);
        let f = fixture(&origin, "disabled");
        let result = f
            .host
            .run(
                input_without_compaction(),
                &Resolver::new(false),
                f.files.clone(),
                &ignore,
            )
            .await;
        assert_eq!(result.is_err(), kind == "failed");
        assert!(!f.root.join("forbidden.txt").exists());
        assert!(!messages(&f.store).iter().any(|m| m["role"] == "toolResult"));
        assert_eq!(server.join().unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xml_synthetic_calls_require_assistant_commit_ack_before_any_effect() {
    struct AssistantFault(ag1_storage::Boundary);
    impl ag1_storage::FaultInjector for AssistantFault {
        fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
            Ok(())
        }
        fn check_journal_command(
            &self,
            command: &str,
            point: ag1_storage::Boundary,
        ) -> ag1_storage::StorageResult<()> {
            if command.ends_with("-assistant-1") && point == self.0 {
                return Err(ag1_storage::StorageError::Injected(point));
            }
            Ok(())
        }
    }
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let source = "<write><path>forbidden.txt</path><content>must wait for acknowledgement</content></write>";
        let (origin, server) = server(vec![response(source)]);
        let f = fixture_configured(
            &origin,
            "disabled",
            vec![],
            Some(Arc::new(AssistantFault(point))),
            None,
            1,
        );
        let error = f
            .host
            .run(
                input_without_compaction(),
                &Resolver::new(false),
                f.files.clone(),
                &ignore,
            )
            .await
            .err()
            .expect("assistant receipt must fail");
        assert!(!f.root.join("forbidden.txt").exists());
        assert!(f.host.settled().unwrap(), "work drained: {error:?}");
        assert!(
            !f.host.persistence_healthy().unwrap(),
            "receipt requires recovery: {error:?}"
        );
        assert_eq!(server.join().unwrap().len(), 1);
        let raw = provider_outputs(&f.store);
        assert!(raw[0].to_string().contains(source));
        assert_eq!(
            messages(&f.store)
                .iter()
                .filter(|m| m["nativeXmlFallback"].is_object())
                .count(),
            usize::from(point == ag1_storage::Boundary::BeforeAck)
        );
        let resolver = Resolver::new(false);
        assert!(
            f.host
                .run(
                    input_without_compaction(),
                    &resolver,
                    f.files.clone(),
                    &ignore
                )
                .await
                .is_err()
        );
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    }
}
