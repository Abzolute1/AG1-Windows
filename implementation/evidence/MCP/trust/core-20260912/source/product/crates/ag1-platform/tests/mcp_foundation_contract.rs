use ag1_platform::mcp::{
    Revision,
    catalog::{CatalogBinding, CatalogScope, CatalogSlot, SchemaSupport},
    wire::{Limits, MessageKind, StdioReader, decode_json, decode_message},
};
use serde_json::{Value, json};
use std::io::{BufReader, Cursor};

fn binding() -> CatalogBinding {
    CatalogBinding {
        server_id: "26fb3347-af67-4a1d-92ec-a915db39c5d0".into(),
        scope: CatalogScope::GeneralChat,
        config_revision: 1,
        auth_profile: "unauthenticated".into(),
        credential_generation: 0,
        connection_generation: 1,
        protocol: Revision::Current20260728,
    }
}
fn tool(name: &str) -> Value {
    json!({"name":name,"description":"Owned fixture", "inputSchema": {
        "type":"object", "properties":{"count":{"type":"integer"}},
        "required":["count"], "additionalProperties":false
    }})
}
fn bytes(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}
fn publish(slot: &mut CatalogSlot, tools: Vec<Value>) {
    let mut refresh = slot.begin_refresh().unwrap();
    refresh.push_page(&bytes(json!({"tools":tools}))).unwrap();
    slot.publish(refresh).unwrap();
}

#[test]
fn supported_revision_labels_are_explicit_not_negotiation() {
    for text in ["2025-11-25", "2026-07-28"] {
        assert_eq!(Revision::parse(text).unwrap().as_str(), text);
    }
    assert!(Revision::parse("2024-11-05").is_err());
    assert!(Revision::parse("latest").is_err());
}

#[test]
fn duplicate_keys_are_rejected_at_every_depth_including_escaped_names() {
    for raw in [
        r#"{"id":1,"id":2}"#,
        r#"{"a":{"x":1,"\u0078":2}}"#,
        r#"[{"secret":"synthetic-private-value","secret":"replacement"}]"#,
    ] {
        let error = decode_json(raw.as_bytes(), Limits::default()).unwrap_err();
        assert!(!format!("{error:?}").contains("synthetic-private-value"));
    }
}

#[test]
fn byte_depth_node_and_utf8_limits_apply_during_intake() {
    assert!(decode_json(b"true", Limits::new(4, 1, 1).unwrap()).is_ok());
    assert!(decode_json(b"false", Limits::new(4, 1, 1).unwrap()).is_err());
    assert!(decode_json(b"[0,1]", Limits::new(100, 2, 3).unwrap()).is_ok());
    assert!(decode_json(b"[0,1,2]", Limits::new(100, 2, 3).unwrap()).is_err());
    assert!(decode_json(b"[[[0]]]", Limits::new(100, 2, 10).unwrap()).is_err());
    assert!(decode_json(&[b'"', 0xff, b'"'], Limits::default()).is_err());
    assert!(decode_json(b"{} {}", Limits::default()).is_err());
    assert!(Limits::new(2 * 1024 * 1024, 32, 20_000).is_err());
    assert!(Limits::new(1, 0, 1).is_err());
}

#[test]
fn rpc_envelopes_classify_without_executing_server_requests() {
    for (body, kind) in [
        (
            json!({"jsonrpc":"2.0","id":"a","method":"sampling/createMessage","params":{}}),
            MessageKind::Request,
        ),
        (
            json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}),
            MessageKind::Notification,
        ),
        (
            json!({"jsonrpc":"2.0","id":1,"result":{"isError":true}}),
            MessageKind::Response,
        ),
        (
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse"}}),
            MessageKind::Error,
        ),
    ] {
        let message = decode_message(&bytes(body.clone()), Limits::default()).unwrap();
        assert_eq!(message.kind, kind);
        assert_eq!(message.body, body);
    }
}

#[test]
fn malformed_rpc_batches_and_ambiguous_responses_are_refused() {
    for body in [
        json!([]),
        json!({"jsonrpc":"1.0","id":1,"result":{}}),
        json!({"jsonrpc":"2.0","id":null,"method":"x"}),
        json!({"jsonrpc":"2.0","id":1.5,"result":{}}),
        json!({"jsonrpc":"2.0","id":1,"method":"x","params":[]}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"error":{}}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":"oops","message":"x"}}),
        json!({"jsonrpc":"2.0","result":{}}),
    ] {
        assert!(
            decode_message(&bytes(body.clone()), Limits::default()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn stdio_frames_handle_fragmentation_crlf_and_multiple_messages() {
    let first = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
    let second = r#"{"jsonrpc":"2.0","method":"ping"}"#;
    let data = format!("{first}\r\n{second}\n");
    let reader = BufReader::with_capacity(3, Cursor::new(data));
    let mut reader = StdioReader::new(reader, Limits::default());
    assert_eq!(reader.read().unwrap().unwrap().kind, MessageKind::Response);
    assert_eq!(
        reader.read().unwrap().unwrap().kind,
        MessageKind::Notification
    );
    assert!(reader.read().unwrap().is_none());
}

#[test]
fn stdio_truncation_flood_and_invalid_output_retire_reader() {
    for data in [
        vec![b'x'; 4096],
        b"{}".to_vec(),
        b"not JSON\n{\"jsonrpc\":\"2.0\",\"method\":\"ping\"}\n".to_vec(),
    ] {
        let mut reader = StdioReader::new(Cursor::new(data), Limits::new(128, 32, 100).unwrap());
        assert!(reader.read().is_err());
        assert!(reader.read().is_err());
    }
}

#[test]
fn catalogs_publish_only_complete_pages_and_preserve_native_binding() {
    let mut slot = CatalogSlot::new(binding()).unwrap();
    let mut refresh = slot.begin_refresh().unwrap();
    assert_eq!(
        refresh
            .push_page(&bytes(json!({"tools":[tool("a")],"nextCursor":"opaque"})))
            .unwrap(),
        Some("opaque".into())
    );
    assert!(slot.current().is_none());
    refresh
        .push_page(&bytes(json!({"tools":[tool("b")]})))
        .unwrap();
    slot.publish(refresh).unwrap();
    let catalog = slot.current().unwrap();
    assert_eq!(catalog.tools().count(), 2);
    assert_eq!(catalog.binding(), &binding());
    assert!(catalog.get("a").unwrap().wire_name().starts_with("mcp_"));
    assert_eq!(catalog.get("a").unwrap().wire_name().len(), 60);
}

#[test]
fn incomplete_poisoned_and_cross_owner_refreshes_cannot_publish() {
    let mut slot = CatalogSlot::new(binding()).unwrap();
    let mut refresh = slot.begin_refresh().unwrap();
    refresh
        .push_page(&bytes(json!({"tools":[],"nextCursor":"more"})))
        .unwrap();
    assert!(slot.publish(refresh).is_err());
    let mut refresh = slot.begin_refresh().unwrap();
    assert!(refresh.push_page(br#"{"tools":[{"name":"bad"}]}"#).is_err());
    assert!(refresh.push_page(&bytes(json!({"tools":[]}))).is_err());
    assert!(slot.publish(refresh).is_err());
    let mut other = CatalogSlot::new(binding()).unwrap();
    let mut foreign = other.begin_refresh().unwrap();
    foreign.push_page(&bytes(json!({"tools":[]}))).unwrap();
    assert!(slot.publish(foreign).is_err());
}

#[test]
fn refresh_revoke_and_identity_changes_fence_stale_results() {
    let mut slot = CatalogSlot::new(binding()).unwrap();
    publish(&mut slot, vec![tool("original")]);
    let mut old = slot.begin_refresh().unwrap();
    assert!(slot.current().is_none());
    old.push_page(&bytes(json!({"tools":[tool("stale")]})))
        .unwrap();
    publish(&mut slot, vec![tool("fresh")]);
    assert!(slot.publish(old).is_err());
    assert!(slot.current().unwrap().get("fresh").is_some());
    for variant in 0..6 {
        let mut pending = slot.begin_refresh().unwrap();
        pending.push_page(&bytes(json!({"tools":[]}))).unwrap();
        let mut next = binding();
        match variant {
            0 => next.config_revision += 1,
            1 => next.credential_generation += 1,
            2 => next.connection_generation += 1,
            3 => next.scope = CatalogScope::Workspace("canonical-project-b".into()),
            4 => next.auth_profile = "different-account".into(),
            _ => next.server_id = "beac3b37-4313-49c5-ab7d-5f95cbfe26f0".into(),
        }
        slot.rebind(next).unwrap();
        assert!(slot.publish(pending).is_err());
        assert!(slot.current().is_none());
    }
}

#[test]
fn repeated_cursors_duplicate_tools_and_extra_pages_poison_refresh() {
    for duplicate_name in [false, true] {
        let mut slot = CatalogSlot::new(binding()).unwrap();
        let mut refresh = slot.begin_refresh().unwrap();
        refresh
            .push_page(&bytes(json!({"tools":[tool("a")],"nextCursor":"same"})))
            .unwrap();
        let page = if duplicate_name {
            json!({"tools":[tool("a")]})
        } else {
            json!({"tools":[tool("b")],"nextCursor":"same"})
        };
        assert!(refresh.push_page(&bytes(page)).is_err());
        assert!(slot.publish(refresh).is_err());
    }
    let mut slot = CatalogSlot::new(binding()).unwrap();
    let mut refresh = slot.begin_refresh().unwrap();
    refresh.push_page(br#"{"tools":[]}"#).unwrap();
    assert!(refresh.push_page(br#"{"tools":[]}"#).is_err());
    assert!(slot.publish(refresh).is_err());
}

#[test]
fn unsupported_constraints_remain_inspectable_without_weakening() {
    for constraint in [
        json!({"$ref":"https://private.invalid/schema"}),
        json!({"oneOf":[{"type":"integer"},{"type":"string"}]}),
        json!({"type":"integer","minimum":2}),
        json!({"type":"string","pattern":"a+"}),
    ] {
        let mut row = tool("unsafe-schema");
        row["inputSchema"]["properties"]["count"] = constraint;
        row["annotations"] = json!({"readOnlyHint":true});
        let mut slot = CatalogSlot::new(binding()).unwrap();
        publish(&mut slot, vec![row.clone()]);
        let tool = slot.current().unwrap().get("unsafe-schema").unwrap();
        assert_eq!(tool.original(), &row);
        assert_eq!(tool.schema_support(), SchemaSupport::UnsupportedInput);
        assert!(tool.validate_arguments(br#"{"count":2}"#).is_err());
    }
}

#[test]
fn strict_arguments_never_coerce_or_ignore_object_constraints() {
    let mut slot = CatalogSlot::new(binding()).unwrap();
    publish(&mut slot, vec![tool("count")]);
    let tool = slot.current().unwrap().get("count").unwrap();
    assert_eq!(tool.schema_support(), SchemaSupport::Supported);
    assert_eq!(
        tool.validate_arguments(br#"{"count":2}"#).unwrap(),
        json!({"count":2})
    );
    for args in [
        br#"{"count":"2"}"#.as_slice(),
        br#"{}"#,
        br#"{"count":2,"extra":1}"#,
        br#"{"count":2,"count":3}"#,
    ] {
        assert!(tool.validate_arguments(args).is_err());
    }
}

#[test]
fn output_schema_shapes_follow_selected_revision_and_are_validated() {
    for shape in [
        json!({"type":"string"}),
        json!({"type":"array","items":{"type":"string"}}),
    ] {
        let mut row = tool("output");
        row["outputSchema"] = shape.clone();
        let mut slot = CatalogSlot::new(binding()).unwrap();
        publish(&mut slot, vec![row.clone()]);
        let tool = slot.current().unwrap().get("output").unwrap();
        assert_eq!(tool.schema_support(), SchemaSupport::Supported);
        let output = if shape["type"] == "string" {
            json!("value")
        } else {
            json!(["value"])
        };
        assert_eq!(
            tool.validate_structured_output(&bytes(output.clone()))
                .unwrap(),
            output
        );
        assert!(tool.validate_structured_output(b"42").is_err());
        let mut legacy = binding();
        legacy.protocol = Revision::Legacy20251125;
        let mut slot = CatalogSlot::new(legacy).unwrap();
        publish(&mut slot, vec![row]);
        assert_eq!(
            slot.current()
                .unwrap()
                .get("output")
                .unwrap()
                .schema_support(),
            SchemaSupport::UnsupportedOutput
        );
    }
}

#[test]
fn thousand_tool_catalog_is_bounded_and_not_a_model_registry() {
    let mut slot = CatalogSlot::new(binding()).unwrap();
    publish(
        &mut slot,
        (0..1000).map(|i| tool(&format!("tool-{i}"))).collect(),
    );
    assert_eq!(slot.current().unwrap().tools().count(), 1000);
    let mut refresh = slot.begin_refresh().unwrap();
    assert!(
        refresh
            .push_page(&bytes(
                json!({"tools":(0..1025).map(|i| tool(&format!("tool-{i}"))).collect::<Vec<_>>()})
            ))
            .is_err()
    );
    assert!(slot.publish(refresh).is_err());
    assert!(slot.current().is_none());
}

#[test]
fn page_budget_is_aggregate_across_empty_pages() {
    let mut slot = CatalogSlot::new(binding()).unwrap();
    let mut refresh = slot.begin_refresh().unwrap();
    for n in 0..31 {
        refresh
            .push_page(&bytes(
                json!({"tools":[],"nextCursor":format!("cursor-{n}")}),
            ))
            .unwrap();
    }
    assert!(
        refresh
            .push_page(&bytes(json!({"tools":[],"nextCursor":"overflow"})))
            .is_err()
    );
    assert!(slot.publish(refresh).is_err());
}

#[test]
fn catalog_byte_budget_includes_metadata_on_all_pages() {
    let mut slot = CatalogSlot::new(binding()).unwrap();
    let mut refresh = slot.begin_refresh().unwrap();
    let padding = "x".repeat(900_000);
    for n in 0..9 {
        refresh
            .push_page(&bytes(
                json!({"tools":[],"nextCursor":format!("page-{n}"),"_meta":{"padding":padding}}),
            ))
            .unwrap();
    }
    assert!(
        refresh
            .push_page(&bytes(json!({"tools":[],"_meta":{"padding":padding}})))
            .is_err()
    );
    assert!(slot.publish(refresh).is_err());
}

#[test]
fn equivalent_object_order_has_identical_catalog_digest() {
    let mut first = CatalogSlot::new(binding()).unwrap();
    let mut second = CatalogSlot::new(binding()).unwrap();
    for (slot, raw) in [
        (
            &mut first,
            br#"{"tools":[{"name":"a","inputSchema":{"type":"object","properties":{}}}]}"#
                .as_slice(),
        ),
        (
            &mut second,
            br#"{"tools":[{"inputSchema":{"properties":{},"type":"object"},"name":"a"}]}"#
                .as_slice(),
        ),
    ] {
        let mut refresh = slot.begin_refresh().unwrap();
        refresh.push_page(raw).unwrap();
        slot.publish(refresh).unwrap();
    }
    assert_eq!(
        first.current().unwrap().get("a").unwrap().digest(),
        second.current().unwrap().get("a").unwrap().digest()
    );
}

#[test]
fn canonical_digest_and_namespace_separate_server_identity_from_display_names() {
    let mut first = CatalogSlot::new(binding()).unwrap();
    publish(&mut first, vec![tool("same-name")]);
    let original = first.current().unwrap().get("same-name").unwrap();
    let name = original.wire_name().to_owned();
    let digest = original.digest().to_owned();
    let mut row = tool("same-name");
    row["description"] = json!("Changed instructions");
    publish(&mut first, vec![row]);
    let changed = first.current().unwrap().get("same-name").unwrap();
    assert_eq!(changed.wire_name(), name);
    assert_ne!(changed.digest(), digest);
    let mut other = binding();
    other.server_id = "beac3b37-4313-49c5-ab7d-5f95cbfe26f0".into();
    let mut second = CatalogSlot::new(other).unwrap();
    publish(&mut second, vec![tool("same-name")]);
    assert_ne!(
        second
            .current()
            .unwrap()
            .get("same-name")
            .unwrap()
            .wire_name(),
        name
    );
}
