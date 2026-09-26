use ag1_core::tools::{
    mcp::{Binding, Identity, Kind, Scope, SelectedDefinition},
    registry::Registry,
};
use serde_json::{Value, json};
fn binding() -> Binding {
    Binding {
        server_id: "26fb3347-af67-4a1d-92ec-a915db39c5d0".into(),
        scope: Scope::GeneralChat,
        config_revision: 1,
        auth_profile: "mcp-unauthenticated".into(),
        credential_generation: 0,
        connection_generation: 1,
        protocol: "2026-07-28".into(),
    }
}
fn capability(kind: Kind, value: Value) -> SelectedDefinition {
    SelectedDefinition::capability(binding(), kind, &value).unwrap()
}
#[test]
fn kind_separates_namespaces_and_tool_identity_serialization_remains_compatible() {
    let tool = capability(
        Kind::Tool,
        json!({"name":"owned","inputSchema":{"type":"object"}}),
    );
    let prompt = capability(Kind::Prompt, json!({"name":"owned"}));
    assert_ne!(tool.identity().wire_name(), prompt.identity().wire_name());
    assert_ne!(
        tool.identity().target_revision(),
        prompt.identity().target_revision()
    );
    let old = serde_json::to_value(tool.identity()).unwrap();
    assert!(old.get("kind").is_none());
    assert_eq!(
        serde_json::from_value::<Identity>(old).unwrap(),
        *tool.identity()
    );
    let mut forged = serde_json::to_value(prompt.identity()).unwrap();
    assert_eq!(forged["kind"], "prompt");
    forged["kind"] = json!("arbitrary-method");
    assert!(serde_json::from_value::<Identity>(forged).is_err());
    let mut registry = Registry::builtins().unwrap();
    registry.replace_mcp_selection(&[tool, prompt]).unwrap();
    assert_eq!(registry.active_names().len(), 2);
}
#[test]
fn resource_uri_and_prompt_name_are_bound_and_arguments_are_strict() {
    let resource = capability(
        Kind::Resource,
        json!({"name":"owned","uri":"file:///private/no-grant"}),
    );
    assert_eq!(
        resource.protocol_params(&json!({})).unwrap(),
        json!({"uri":"file:///private/no-grant"})
    );
    assert!(
        resource
            .protocol_params(&json!({"uri":"https://other"}))
            .is_err()
    );
    let prompt = capability(
        Kind::Prompt,
        json!({"name":"owned","arguments":[{"name":"topic","required":true},{"name":"optional"}]}),
    );
    assert_eq!(
        prompt.protocol_params(&json!({"topic":"text"})).unwrap(),
        json!({"name":"owned","arguments":{"topic":"text"}})
    );
    for args in [
        json!({}),
        json!({"topic":1}),
        json!({"topic":"x","name":"other"}),
        json!({"topic":"x","inputResponses":{}}),
        json!({"topic":"x".repeat(4097)}),
    ] {
        assert!(prompt.protocol_params(&args).is_err());
    }
    for original in [
        json!({"name":"owned","arguments":[{"name":"x"},{"name":"x"}]}),
        json!({"name":"owned","arguments":[{"name":"x","required":"yes"}]}),
    ] {
        assert!(SelectedDefinition::capability(binding(), Kind::Prompt, &original).is_err());
    }
}
#[test]
fn templates_encode_explicit_strings_and_refuse_unsupported_expressions_and_bombs() {
    let template = capability(
        Kind::ResourceTemplate,
        json!({"name":"owned","uriTemplate":"fixture://host/{x}/{x}?q={y}"}),
    );
    assert_eq!(
        template
            .protocol_params(&json!({"x":"/ é%","y":"?&=~"}))
            .unwrap()["uri"],
        "fixture://host/%2F%20%C3%A9%25/%2F%20%C3%A9%25?q=%3F%26%3D~"
    );
    for args in [
        json!({"x":"a"}),
        json!({"x":"a","y":[]}),
        json!({"x":"a","y":"b","uri":"elsewhere"}),
        json!({"x":"é".repeat(1000),"y":"a"}),
    ] {
        assert!(template.protocol_params(&args).is_err());
    }
    for uri in [
        "fixture://host/{+x}",
        "fixture://host/{x,y}",
        "fixture://host/{x*}",
        "fixture://host/{x:2}",
        "fixture://host/{x",
        "fixture://host/x}",
        "fixture://host/static",
        "relative/{x}",
    ] {
        assert!(
            SelectedDefinition::capability(
                binding(),
                Kind::ResourceTemplate,
                &json!({"name":"owned","uriTemplate":uri})
            )
            .is_err(),
            "{uri}"
        );
    }
}
#[test]
fn metadata_and_scope_changes_invalidate_prepared_content_calls() {
    for kind in [Kind::Resource, Kind::ResourceTemplate, Kind::Prompt] {
        let mut raw = match kind {
            Kind::Resource => json!({"name":"owned","uri":"fixture://host/source"}),
            Kind::ResourceTemplate => json!({"name":"owned","uriTemplate":"fixture://host/{x}"}),
            _ => json!({"name":"owned"}),
        };
        let selected = capability(kind, raw.clone());
        let mut registry = Registry::builtins().unwrap();
        registry.replace_mcp_selection(&[selected.clone()]).unwrap();
        let args = if kind == Kind::ResourceTemplate {
            json!({"x":"a"})
        } else {
            json!({})
        };
        let call = registry
            .prepare("op", "call", &selected.identity().wire_name(), &args, true)
            .unwrap();
        raw["description"] = json!("changed metadata");
        let changed = capability(kind, raw.clone());
        assert_eq!(
            changed.identity().wire_name(),
            selected.identity().wire_name()
        );
        registry.replace_mcp_selection(&[changed]).unwrap();
        assert!(registry.validate_current(&call).is_err());
        let mut other = binding();
        other.scope = Scope::Workspace("other".into());
        assert_ne!(
            SelectedDefinition::capability(other, kind, &raw)
                .unwrap()
                .identity()
                .target_revision(),
            selected.identity().target_revision()
        );
    }
}

#[test]
fn template_approval_displays_exact_expanded_uri_and_still_requires_explicit_answer() {
    use ag1_core::tools::{
        approval::{Authorizer, Preflight, PreflightContext},
        policy::Policy,
        registry::Scope as ExecutionScope,
    };
    let selected = capability(
        Kind::ResourceTemplate,
        json!({"name":"owned","uriTemplate":"fixture://host/{path}"}),
    );
    let mut registry = Registry::builtins().unwrap();
    registry.replace_mcp_selection(&[selected.clone()]).unwrap();
    let call = registry
        .prepare(
            "op",
            "call",
            &selected.identity().wire_name(),
            &json!({"path":"a/b"}),
            true,
        )
        .unwrap();
    let scope = ExecutionScope {
        workspace: "workspace".into(),
        session: "session".into(),
        epoch: 1,
        revision: 0,
    };
    let target = selected.identity().target_revision();
    let policy: Policy = serde_json::from_value(json!({"mode":"disabled","cwd":""})).unwrap();
    let Preflight::Approval(request) = Authorizer::new("owner")
        .unwrap()
        .preflight(
            call,
            PreflightContext {
                scope: &scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: &target,
                now: 100,
                ttl: 10000,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!("resource read must ask")
    };
    assert_eq!(request.display["request"]["uri"], "fixture://host/a%2Fb");
    assert!(
        request.display["details"]
            .as_str()
            .unwrap()
            .contains("fixture://host/a%2Fb")
    );
    assert_eq!(request.display["risk"], "unknown");
}
