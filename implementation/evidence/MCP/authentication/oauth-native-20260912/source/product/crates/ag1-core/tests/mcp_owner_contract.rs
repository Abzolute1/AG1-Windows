use ag1_core::tools::{
    mcp::{Binding, ExecutionScope, Scope as CatalogScope, SelectedDefinition},
    registry::Scope,
};
use serde_json::json;

#[test]
fn canonical_project_and_native_execution_owner_are_both_bound_without_peer_authority() {
    let binding = Binding {
        server_id: "26fb3347-af67-4a1d-92ec-a915db39c5d0".into(),
        scope: CatalogScope::Workspace("project".into()),
        config_revision: 1,
        auth_profile: "mcp-unauthenticated".into(),
        credential_generation: 0,
        connection_generation: 1,
        protocol: "2026-07-28".into(),
    };
    let scope = Scope {
        workspace: "conversation-owner".into(),
        session: "chat".into(),
        epoch: 3,
        revision: 8,
    };
    let original = json!({"name":"owned","inputSchema":{"type":"object"},"executionScope":{"workspace":"conversation-owner","session":"chat","epoch":3}});
    let raw = SelectedDefinition::new(binding.clone(), &original).unwrap();
    assert!(raw.identity().execution_scope.is_none());
    assert!(raw.identity().check_scope(&scope).is_err());
    let native = raw
        .clone()
        .for_execution_scope(Some(&ExecutionScope::from_native(&scope).unwrap()))
        .unwrap();
    assert_eq!(native.identity().binding.scope, binding.scope);
    assert_ne!(
        native.identity().target_revision(),
        raw.identity().target_revision()
    );
    native.identity().check_scope(&scope).unwrap();
    native
        .identity()
        .check_scope(&Scope {
            revision: 99,
            ..scope.clone()
        })
        .unwrap();
    for foreign in [
        Scope {
            workspace: "project".into(),
            ..scope.clone()
        },
        Scope {
            session: "other".into(),
            ..scope.clone()
        },
        Scope {
            epoch: 4,
            ..scope.clone()
        },
    ] {
        assert!(native.identity().check_scope(&foreign).is_err());
    }
    assert!(ExecutionScope::from_native(&Scope { epoch: 0, ..scope }).is_err());
}
