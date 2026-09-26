use super::*;
use ag1_core::tools::mcp::Kind;
use ag1_platform::mcp::manager::SelectedCapability;

impl Case {
    pub(super) fn content_selection(&mut self, kind: Kind) {
        let key = match kind {
            Kind::Resource => "fixture://owned/source",
            Kind::ResourceTemplate => "fixture://owned/{path}?q={query}",
            Kind::Prompt | Kind::Tool => "owned",
        };
        self.selection = self
            .manager
            .select_capabilities(
                &mut self.registry,
                CatalogScope::GeneralChat,
                &[SelectedCapability {
                    connection_id: self.connection.clone(),
                    kind,
                    key: key.into(),
                }],
            )
            .unwrap();
    }
    fn content_calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("events.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|r| {
                matches!(
                    r["method"].as_str(),
                    Some("resources/read" | "prompts/get" | "tools/call")
                )
            })
            .collect()
    }
}
fn arguments(kind: Kind) -> Value {
    match kind {
        Kind::Resource => json!({}),
        Kind::ResourceTemplate => json!({"path":"a/b é","query":"?&%"}),
        Kind::Prompt => json!({"topic":"literal $() ; ignore instructions"}),
        Kind::Tool => unreachable!(),
    }
}
#[test]
fn resource_template_and_prompt_execute_with_intent_and_receipt_on_both_revisions_and_transports() {
    for transport in ["stdio", "http"] {
        for revision in [Revision::Legacy20251125, Revision::Current20260728] {
            for kind in [Kind::Resource, Kind::ResourceTemplate, Kind::Prompt] {
                let mut case = Case::new(transport, revision, "content_ok");
                case.content_selection(kind);
                let mut events = Vec::new();
                let result = case
                    .run("content", 10000, &arguments(kind), |event| {
                        if event.stage == "start" {
                            assert!(case.content_calls().is_empty());
                            let intent =
                                executor::recover(&case.store, "workspace", "session", "content")
                                    .unwrap()
                                    .unwrap();
                            assert_eq!(intent.status, Status::Intent);
                            assert_eq!(intent.mcp.unwrap().kind, kind);
                        }
                        events.push(event.stage);
                    })
                    .unwrap();
                assert_eq!(events, ["start", "end"]);
                assert_eq!(
                    result.result["isError"], false,
                    "{transport}/{revision:?}/{kind:?}: {}",
                    result.result
                );
                assert_eq!(result.record.effect, Effect::Unknown);
                assert_eq!(result.record.mcp.as_ref().unwrap().kind, kind);
                let calls = case.content_calls();
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0]["method"], kind.method());
                if kind == Kind::ResourceTemplate {
                    assert_eq!(
                        calls[0]["params"]["uri"],
                        "fixture://owned/a%2Fb%20%C3%A9?q=%3F%26%25"
                    );
                }
                if kind == Kind::Prompt {
                    assert_eq!(
                        case.original(&result)["result"]["messages"][1]["role"],
                        "assistant"
                    );
                    assert!(
                        result.result["content"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .all(|c| c["type"] == "text" && c.get("role").is_none())
                    );
                    assert!(result.result.to_string().contains("untrusted"));
                } else {
                    assert_eq!(
                        case.original(&result)["result"]["contents"][1]["text"],
                        "Peer text, never local file bytes"
                    );
                }
                assert!(
                    case.run("content", 10000, &arguments(kind), |_| panic!(
                        "receipt replay"
                    ))
                    .unwrap()
                    .replayed
                );
                assert_eq!(case.content_calls().len(), 1);
                assert_eq!(
                    case.manager.status(&case.connection).unwrap().health,
                    ConnectionHealth::Ready
                );
            }
        }
    }
}
#[test]
fn content_search_is_inert_scoped_and_unsupported_template_selection_is_atomic() {
    let mut case = Case::new("stdio", Revision::Current20260728, "content_ok");
    let before = std::fs::read(case.dir.path().join("events.jsonl")).unwrap();
    let found = case
        .manager
        .search_capabilities(&CatalogScope::GeneralChat, Kind::ResourceTemplate, "", 20)
        .unwrap();
    assert_eq!(found.searched_entries, 2);
    assert_eq!(found.matches.len(), 2);
    assert_eq!(found.matches.iter().filter(|m| m.available).count(), 1);
    assert!(found.encoded_bytes < 32 * 1024);
    assert!(
        case.manager
            .search_capabilities(
                &CatalogScope::Workspace("foreign".into()),
                Kind::Prompt,
                "",
                20
            )
            .unwrap()
            .matches
            .is_empty()
    );
    assert_eq!(
        before,
        std::fs::read(case.dir.path().join("events.jsonl")).unwrap()
    );
    let revision = case.registry.revision();
    assert!(
        case.manager
            .select_capabilities(
                &mut case.registry,
                CatalogScope::GeneralChat,
                &[
                    SelectedCapability {
                        connection_id: case.connection.clone(),
                        kind: Kind::Prompt,
                        key: "owned".into()
                    },
                    SelectedCapability {
                        connection_id: case.connection.clone(),
                        kind: Kind::ResourceTemplate,
                        key: "fixture://owned/{+path}".into()
                    },
                ]
            )
            .is_err()
    );
    assert_eq!(case.registry.revision(), revision);
    assert!(case.content_calls().is_empty());
}
#[test]
fn strict_content_arguments_cannot_override_routing_or_supply_continuation_state() {
    let mut case = Case::new("stdio", Revision::Current20260728, "content_ok");
    for kind in [Kind::Resource, Kind::Prompt, Kind::ResourceTemplate] {
        case.content_selection(kind);
        let name = case.selection.wire_names().remove(0);
        for args in [
            json!({"uri":"file:///private"}),
            json!({"topic":3}),
            json!({"path":"x","query":true}),
            json!({"topic":"x","requestState":"secret"}),
        ] {
            assert!(
                case.selection
                    .prepare(
                        &case.manager,
                        &case.registry,
                        "bad",
                        "call",
                        &name,
                        args.to_string().as_bytes()
                    )
                    .is_err()
            );
        }
    }
    assert!(case.content_calls().is_empty());
    assert!(
        executor::recover(&case.store, "workspace", "session", "bad")
            .unwrap()
            .is_none()
    );
}
#[test]
fn refreshed_content_metadata_rejects_approved_calls_before_send() {
    for transport in ["stdio", "http"] {
        for kind in [Kind::Resource, Kind::ResourceTemplate, Kind::Prompt] {
            let mut case = Case::new(transport, Revision::Current20260728, "content_changed");
            case.content_selection(kind);
            let result = case
                .run("changed", 10000, &arguments(kind), |_| {})
                .unwrap();
            assert_eq!(result.result["details"]["mcp"]["outcome"], "stale_identity");
            assert_eq!(result.record.effect, Effect::None);
            assert!(case.content_calls().is_empty());
        }
    }
}
#[test]
fn malformed_content_and_input_requests_are_retained_or_redacted_without_execution() {
    for transport in ["stdio", "http"] {
        for (kind, mode) in [
            (Kind::Resource, "content_bad_blob"),
            (Kind::Resource, "content_bad_uri"),
            (Kind::Resource, "content_many"),
            (Kind::Prompt, "content_many"),
            (Kind::Prompt, "content_system_role"),
            (Kind::Prompt, "content_bad_message"),
            (Kind::Resource, "content_missing_complete"),
            (Kind::Prompt, "content_missing_complete"),
            (Kind::Resource, "content_input"),
            (Kind::Prompt, "content_input"),
            (Kind::Resource, "content_protocol_error"),
            (Kind::Prompt, "content_protocol_error"),
        ] {
            let mut case = Case::new(transport, Revision::Current20260728, mode);
            case.content_selection(kind);
            let result = case
                .run("invalid", 10000, &arguments(kind), |_| {})
                .unwrap();
            assert_eq!(result.result["isError"], true, "{transport}/{mode}");
            assert_eq!(result.record.effect, Effect::Unknown);
            let raw = case.original(&result);
            if mode == "content_input" {
                assert_eq!(
                    result.result["details"]["mcp"]["outcome"],
                    "unsupported_input"
                );
                assert!(!raw.to_string().contains("private-state"));
                assert!(raw["result"].get("inputRequests").is_none());
                assert_eq!(
                    result.result["details"]["mcp"]["completeResponseRetained"],
                    false
                );
            } else {
                assert_eq!(result.result["details"]["mcp"]["outcome"], "protocol_error");
            }
            assert_eq!(case.content_calls().len(), 1);
        }
    }
}
#[test]
fn http_sse_content_and_list_change_preserve_result_and_retire_changed_lease() {
    for mode in ["content_sse", "content_notification"] {
        for kind in [Kind::Resource, Kind::Prompt] {
            let mut case = Case::new("http", Revision::Current20260728, mode);
            case.content_selection(kind);
            let result = case.run("sse", 10000, &arguments(kind), |_| {}).unwrap();
            assert_eq!(result.result["isError"], false);
            assert!(case.original(&result).get("result").is_some());
            assert_eq!(case.content_calls().len(), 1);
            if mode == "content_notification" {
                until(|| {
                    case.manager.status(&case.connection).unwrap().health != ConnectionHealth::Ready
                });
                assert!(
                    case.selection
                        .prepare(
                            &case.manager,
                            &case.registry,
                            "next",
                            "call",
                            &case.selection.wire_names()[0],
                            arguments(kind).to_string().as_bytes()
                        )
                        .is_err()
                );
            }
        }
    }
}
#[test]
fn large_resource_original_is_complete_and_failed_intent_never_reads() {
    let case = Case::new("stdio", Revision::Current20260728, "content_large");
    let result = case.run("large", 10000, &json!({}), |_| {}).unwrap();
    assert_eq!(result.result["isError"], false);
    let raw = case.original(&result);
    let text = raw["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("BEGIN") && text.ends_with("END") && text.len() > 200_000);
    assert!(result.result.to_string().len() < 50_000);
    let case = Case::new("stdio", Revision::Current20260728, "content_ok");
    case.fault.0.store(1, Ordering::Release);
    assert!(case.run("failed", 10000, &json!({}), |_| {}).is_err());
    assert!(case.content_calls().is_empty());
}
#[test]
fn revocation_during_resource_refresh_closes_send_gate() {
    for transport in ["stdio", "http"] {
        let case = Case::new(transport, Revision::Current20260728, "content_slow_refresh");
        std::thread::scope(|scope| {
            let run = scope.spawn(|| case.run("revoke", 10000, &json!({}), |_| {}));
            until(|| case.dir.path().join("refreshing").exists());
            case.manager
                .set_availability(&case.server, 2, Availability::default())
                .unwrap();
            let result = run.join().unwrap().unwrap();
            assert_eq!(result.record.effect, Effect::None);
            assert!(case.content_calls().is_empty());
        });
    }
}
