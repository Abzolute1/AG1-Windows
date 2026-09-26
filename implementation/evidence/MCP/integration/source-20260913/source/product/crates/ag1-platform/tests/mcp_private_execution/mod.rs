use super::*;
use ag1_core::SecretBytes;
use ag1_platform::mcp::auth::{Material, Vault};
use ag1_storage::mcp_private::PrivateStore;
fn authenticated(echo: bool) -> Case {
    let mut case = Case::new("stdio", Revision::Current20260728, "structured");
    case.manager.disconnect(&case.connection).unwrap();
    case.manager.wait_disconnected(&case.connection).unwrap();
    let vault =
        Vault::open(PrivateStore::open(&case.dir.path().join("private-vault")).unwrap()).unwrap();
    case.manager = Arc::new(
        Arc::try_unwrap(case.manager)
            .ok()
            .expect("owned manager")
            .with_vault(vault)
            .unwrap(),
    );
    let mut record = case.manager.records().unwrap().remove(0);
    let wrapper = case.dir.path().join("private-wrapper.py");
    std::fs::write(&wrapper,format!("import os\nfrom pathlib import Path\nsecret=os.environ.pop('TOKEN')\nassert secret=='owned-private-token'\ncode=Path('peer.py').read_text()\n{}\nexec(code)\n",if echo {"code=code.replace(\"'owned result'\",repr(secret))"}else{""})).unwrap();
    if let Configuration::Local {
        arguments,
        secret_environment,
        ..
    } = &mut record.draft.configuration
    {
        arguments[1] = wrapper.to_str().unwrap().into();
        secret_environment.insert("TOKEN".into());
    }
    case.manager
        .edit(&case.server, record.revision, record.draft)
        .unwrap();
    let rev = case.manager.records().unwrap()[0].revision;
    case.manager
        .save_private(
            case.manager.review_private(&case.server, rev).unwrap(),
            Material::Environment(BTreeMap::from([(
                "TOKEN".into(),
                SecretBytes::new(b"owned-private-token".to_vec()),
            )])),
        )
        .unwrap();
    let rev = case.manager.records().unwrap()[0].revision;
    case.manager
        .set_availability(
            &case.server,
            rev,
            Availability {
                enabled: true,
                general_chat: true,
                ..Default::default()
            },
        )
        .unwrap();
    case.connection = case
        .manager
        .connect(
            case.manager
                .review_connection(&case.server, CatalogScope::GeneralChat)
                .unwrap(),
            ConnectionConsent::LocalHostAccountExecution,
        )
        .unwrap();
    until(|| case.manager.status(&case.connection).unwrap().health == ConnectionHealth::Ready);
    case.registry = Registry::builtins().unwrap();
    case.selection = case
        .manager
        .select_tools(
            &mut case.registry,
            CatalogScope::GeneralChat,
            &[SelectedTool {
                connection_id: case.connection.clone(),
                original_name: "owned".into(),
            }],
        )
        .unwrap();
    case
}
#[test]
fn credential_bound_operation_uses_shared_executor_and_logout_rejects_old_ticket() {
    let case = authenticated(false);
    let result = case
        .run("private-success", 1000, &json!({"value":7}), |_| {})
        .unwrap();
    assert_eq!(result.record.status, Status::Completed);
    assert_eq!(case.calls().len(), 1);
    assert!(case.original(&result).to_string().contains("owned result"));
    let pending = case.ticket("private-late", 1000, &json!({"value":7}));
    case.manager
        .logout(&case.server, case.manager.records().unwrap()[0].revision)
        .unwrap();
    assert!(case.execute("private-late", pending, |_| {}).is_err());
    assert_eq!(case.calls().len(), 1);
}
#[test]
fn supplied_secret_echo_is_withheld_before_journal_artifact_or_model_projection() {
    let case = authenticated(true);
    let result = case
        .run("private-echo", 1000, &json!({"value":7}), |_| {})
        .unwrap();
    assert_eq!(result.result["isError"], true);
    assert_eq!(
        result.result["details"]["mcp"]["outcome"],
        "private_output_withheld"
    );
    assert_eq!(result.record.effect, Effect::Unknown);
    assert_eq!(case.calls().len(), 1);
    assert!(!result.result.to_string().contains("owned-private-token"));
    assert!(result.result["details"]["mcp"]["originalResponse"].is_null());
    for entry in std::fs::read_dir(case.dir.path().join("conversation/blobs")).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(
            !bytes
                .windows(b"owned-private-token".len())
                .any(|v| v == b"owned-private-token")
        );
    }
}
