use super::*;
use ag1_platform::mcp::{definitions::LocalAccess, service::Service};
use ag1_core::tools::registry::Scope;
use serde_json::json;
use std::sync::Arc;

#[test]
fn confined_connection_enforces_directory_network_and_loader_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let allowed = dir.path().join("project");
    std::fs::create_dir(&allowed).unwrap();
    let outside = dir.path().join("outside");
    let marker = dir.path().join("loader-ran-before-isolation");
    std::fs::write(&outside, "owned outside").unwrap();
    std::fs::write(allowed.join("allowed"), "owned").unwrap();
    std::os::unix::fs::symlink(&outside, allowed.join("escape")).unwrap();
    std::fs::write(allowed.join("peer.py"), include_str!("../fixtures/mcp_contained.py")).unwrap();
    let preload = allowed.join("owned-preload.so");
    let source = dir.path().join("preload.c");
    std::fs::write(&source, format!("#include <fcntl.h>\n#include <unistd.h>\n__attribute__((constructor)) static void probe(void) {{ int fd=open(\"{}\",O_WRONLY|O_CREAT,0600); if(fd>=0)close(fd); }}", marker.display())).unwrap();
    assert!(std::process::Command::new("/usr/bin/cc").args(["-shared", "-fPIC"]).arg(&source).arg("-o").arg(&preload).status().unwrap().success());
    let m = manager(&dir.path().join("state"));
    let draft = Draft {
        name:"contained fixture".into(), protocol:Revision::Current20260728.as_str().into(), auth_profile:None, provenance:Provenance::Manual,
        configuration:Configuration::Local { executable:"/usr/bin/python3".into(), arguments:vec!["-I".into(),allowed.join("peer.py").to_str().unwrap().into(),outside.to_str().unwrap().into()],cwd:allowed.to_str().unwrap().into(),
            public_environment:BTreeMap::from([("LD_PRELOAD".into(),preload.to_str().unwrap().into()),("OWNED_PRIVATE".into(),"owned-containment-value".into())]),secret_environment:BTreeSet::new() }
    };
    let id = m.add_drafts_with_access(vec![draft.clone()],LocalAccess::ReadOnlyDirectory).unwrap().remove(0);
    assert!(m.connect(m.review_test_connection(&id,CatalogScope::GeneralChat).unwrap(),ConnectionConsent::LocalHostAccountExecution).is_err());
    let connection = m.connect(m.review_test_connection(&id,CatalogScope::GeneralChat).unwrap(),ConnectionConsent::LocalReadOnlyDirectory).unwrap();
    wait(&m,&connection,ConnectionHealth::Ready);
    let catalog = m.inspect_catalog(&connection,&CatalogScope::GeneralChat).unwrap();
    let report: serde_json::Value = serde_json::from_str(catalog.tools[0].original()["description"].as_str().unwrap()).unwrap();
    assert!(report.as_object().unwrap().values().all(|v|v==true),"{report}");
    assert!(!marker.exists(),"server loader settings reached the host launcher");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(),"owned outside");
    assert_eq!(std::fs::read_to_string(allowed.join("allowed")).unwrap(),"owned");
    let old = m.review_test_connection(&id,CatalogScope::GeneralChat).unwrap();
    m.edit_with_access(&id,1,draft,LocalAccess::HostAccount).unwrap();
    assert!(m.connect(old,ConnectionConsent::LocalReadOnlyDirectory).is_err());
    m.wait_disconnected(&connection).unwrap();
    let r=m.records().unwrap().remove(0);
    assert_eq!(r.configuration_revision(),2);
    assert_eq!(r.local_access,LocalAccess::HostAccount);
}

#[test]
fn project_import_requires_native_folder_review_and_unchanged_inactive_snapshot() {
    let dir=tempfile::tempdir().unwrap();
    let project=dir.path().join("project");std::fs::create_dir(&project).unwrap();
    let ws=ag1_platform::workspace::WorkspaceService::new().unwrap().register(&project).unwrap();
    let m=Arc::new(manager(&dir.path().join("state")));
    let service=Service::from_manager(m.clone(),Path::new(env!("CARGO_BIN_EXE_ag1-process-host")).into());
    let scope=Scope {workspace:ws.id().as_str().into(),session:"owned-import".into(),epoch:1,revision:1};
    let catalog_scope=CatalogScope::Workspace(ws.id().as_str().into());
    let session=service.session_with_project(catalog_scope.clone(),scope.clone(),None,Some(ws.clone())).unwrap();
    let other=service.session_with_project(catalog_scope,Scope {session:"other".into(),..scope.clone()},None,Some(ws)).unwrap();
    let bytes=br#"{"mcpServers":{"owned imported":{"command":"/does/not/exist","args":["never executed"]}}}"#;
    std::fs::write(project.join(".mcp.json"),bytes).unwrap();
    assert_eq!(session.dispatch(&scope,&json!({"action":"list"})).unwrap()["projectImportAvailable"],true);
    assert!(m.records().unwrap().is_empty());
    let first=session.dispatch(&scope,&json!({"action":"import_review"})).unwrap();
    assert!(m.records().unwrap().is_empty());
    assert_eq!(first["drafts"][0]["configuration"]["cwd"],project.to_str().unwrap());
    assert!(other.dispatch(&Scope {session:"other".into(),..scope.clone()},&json!({"action":"import_confirm","review":first["review"]})).is_err());
    std::fs::write(project.join(".mcp.json"),b"{}").unwrap();
    assert_eq!(session.dispatch(&scope,&json!({"action":"import_confirm","review":first["review"]})).unwrap_err(),DomainError::Conflict);
    std::fs::write(project.join(".mcp.json"),bytes).unwrap();
    let review=session.dispatch(&scope,&json!({"action":"import_review"})).unwrap();
    let saved=session.dispatch(&scope,&json!({"action":"import_confirm","review":review["review"]})).unwrap();
    assert_eq!(saved["active"],false);
    let record=m.records().unwrap().remove(0);
    assert!(!record.availability.enabled);
    assert_eq!(record.local_access,LocalAccess::ReadOnlyDirectory);
    assert!(matches!(record.draft.provenance,Provenance::Project{..}));
    assert!(session.dispatch(&scope,&json!({"action":"import_confirm","review":review["review"]})).is_err());
    assert!(session.dispatch(&scope,&json!({"action":"import_review","path":"/unreviewed"})).is_err());
    std::fs::remove_file(project.join(".mcp.json")).unwrap();
    let escaped=dir.path().join("outside.json");std::fs::write(&escaped,bytes).unwrap();
    std::os::unix::fs::symlink(escaped,project.join(".mcp.json")).unwrap();
    assert!(session.dispatch(&scope,&json!({"action":"import_review"})).is_err());
    session.close().unwrap();other.close().unwrap();
}
