use ag1_core::{
    DomainError,
    context::{compiler::CompileOptions, prompt::PromptInput},
    runtime::{prompt::PromptConfiguration, session::Configuration},
    tools::{
        mcp::{Binding, Scope as McpScope, SelectedDefinition, discovery},
        registry::Registry,
        schema::Schema,
    },
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn metadata_policy_exception_requires_the_native_host_origin() {
    use ag1_core::tools::{
        approval::{Authorizer, Preflight, PreflightContext},
        policy::{Mode, Policy},
        registry::{Origin, Scope},
    };
    for origin in [Origin::TrustedHost, Origin::Extension] {
        let mut registry = Registry::builtins().unwrap();
        registry.register(discovery::metadata(), origin).unwrap();
        registry
            .activate(&[discovery::NAME.into()], &[discovery::NAME.into()].into())
            .unwrap();
        let call = registry
            .prepare(
                "metadata",
                "call",
                discovery::NAME,
                &json!({"action":"status"}),
                true,
            )
            .unwrap();
        let scope = Scope {
            workspace: "owned".into(),
            session: "session".into(),
            epoch: 1,
            revision: 1,
        };
        let policy = Policy {
            mode: Mode::Headless,
            cwd: "/owned".into(),
            planning_read_only_tools: None,
            trusted_tools: vec![],
            extension_tools: vec![],
        };
        let result = Authorizer::new("owned")
            .unwrap()
            .preflight(
                call,
                PreflightContext {
                    scope: &scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: "mcp-discovery-1",
                    now: 1,
                    ttl: 100,
                    untrusted_extensions: true,
                },
            )
            .unwrap();
        assert_eq!(
            matches!(result, Preflight::Rejected(_)),
            origin == Origin::Extension
        );
        assert!(!matches!(result, Preflight::Approval(_)));
    }
}

#[test]
fn captured_prompt_admits_only_typed_selected_mcp_metadata_and_evicts_it() {
    let mut registry = Registry::builtins().unwrap();
    registry.register_sdk(discovery::metadata()).unwrap();
    let tools: Vec<_> = registry
        .public_tools()
        .into_iter()
        .filter(|t| t["name"] == "read" || t["name"] == discovery::NAME)
        .collect();
    let prompt = PromptConfiguration::new(
        PromptInput {
            date: "2026-09-12".into(),
            cwd: "/owned".into(),
            ..Default::default()
        },
        registry.clone(),
        "a".repeat(64),
    )
    .unwrap();
    let config = Configuration {
        model: json!({"id":"fixture","provider":"fixture","contextWindow":32768}),
        thinking: "off".into(),
        tools: tools.clone(),
        system_prompt: String::new(),
        prompt: Some(Arc::new(prompt.clone())),
        compiler: CompileOptions::default(),
    };
    let selected = SelectedDefinition::new(Binding {server_id:"26fb3347-af67-4a1d-92ec-a915db39c5d0".into(),scope:McpScope::GeneralChat,config_revision:1,auth_profile:"unauthenticated".into(),credential_generation:0,connection_generation:1,protocol:"2026-07-28".into()}, &json!({"name":"owned","description":"Untrusted MCP description","inputSchema":{"type":"object"},"promptGuidelines":["NEVER_ADMIT_THIS_INSTRUCTION"]})).unwrap();
    registry.replace_mcp_selection(&[selected.clone()]).unwrap();
    let copied = registry.mcp_selection();
    assert_eq!(copied.len(), 1);
    assert_eq!(copied[0].identity(), selected.identity());
    let mut view = tools.clone();
    view.push(selected.metadata().clone());
    assert!(
        config.with_tools(view.clone()).is_err(),
        "raw metadata cannot add an uncaptured capability"
    );
    let mut next = config.clone();
    next.prompt = Some(Arc::new(prompt.with_mcp_selection(&copied).unwrap()));
    let next = next.with_tools(view.clone()).unwrap();
    assert!(!next.system_prompt.contains("NEVER_ADMIT_THIS_INSTRUCTION"));
    let mut forged = view.clone();
    forged[0]["description"] = json!("forged native instruction");
    assert_eq!(next.with_tools(forged).unwrap_err(), DomainError::Conflict);
    let mut forged = view;
    forged.last_mut().unwrap()["parameters"] = json!({"type":"string"});
    assert_eq!(next.with_tools(forged).unwrap_err(), DomainError::Conflict);
    let mut evicted = next.clone();
    evicted.prompt = Some(Arc::new(
        next.prompt
            .as_ref()
            .unwrap()
            .with_mcp_selection(&[])
            .unwrap(),
    ));
    evicted.with_tools(tools).unwrap();
    assert!(evicted.with_tools(next.tools).is_err());
}

#[test]
fn facade_schema_has_no_raw_execution_connection_or_authority_arguments() {
    let schema = Schema::parse(&discovery::metadata()["parameters"]).unwrap();
    for good in [
        json!({"action":"search","kind":"tool","query":""}),
        json!({"action":"status"}),
        json!({"action":"clear"}),
        json!({"action":"select","entries":[]}),
    ] {
        schema.validate(&good).unwrap();
    }
    for bad in [
        json!({"action":"tools/call"}),
        json!({"action":"connect"}),
        json!({"action":"search","kind":"tool","query":"x","scope":"other"}),
        json!({"action":"search","kind":"tool","query":"x","limit":21}),
        json!({"action":"select","entries":vec![json!({"connectionId":"x","kind":"tool","key":"x"});17]}),
        json!({"action":"inspect","entry":{"connectionId":"x","kind":"tool","key":"x","inputResponses":{}}}),
    ] {
        assert!(schema.validate(&bad).is_err(), "{bad}");
    }
    assert!(discovery::metadata().to_string().len() < 8192);
}
