use ag1_core::context::retrieval::{
    RecentAccess, RetrievalOptions, SourceRetrievalPort, WorkingTree, build_capsule,
};
use ag1_platform::{
    process::ProcessSupervisor, retrieval::SourceHost, source::SourceService,
    workspace::WorkspaceService,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
fn input() -> Value {
    serde_json::from_str(include_str!(
        "../../../../fixtures/context/source-inputs.json"
    ))
    .unwrap()
}
fn fixture(scenario: &Value) -> (tempfile::TempDir, SourceHost) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let files = input();
    for (name, text) in files["files"].as_object().unwrap() {
        let path = workspace.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text.as_str().unwrap()).unwrap();
    }
    if scenario["long"] == true {
        fs::write(
            workspace.join("src/long.ts"),
            format!(
                "export const longSource = \"{}\";\n{}",
                "x".repeat(5000),
                (0..100)
                    .map(|i| format!("// line {i}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        )
        .unwrap();
    }
    let registry = WorkspaceService::new().unwrap();
    let workspace = registry.register(&workspace).unwrap();
    let recent = scenario["recent"]
        .as_str()
        .map(|p| {
            vec![RecentAccess {
                path: p.into(),
                kind: "file".into(),
                count: 1,
                distance: 0,
            }]
        })
        .unwrap_or_default();
    let changed: BTreeSet<String> = scenario["changed"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| p.as_str().unwrap().into())
        .collect();
    let tree = WorkingTree {
        inside_repository: !changed.is_empty(),
        changed_lines: changed
            .iter()
            .map(|p| (p.clone(), BTreeSet::from([1])))
            .collect(),
        changed_files: changed,
    };
    let host = SourceHost {
        source: SourceService::new(workspace.clone()),
        workspace,
        recent,
        tree,
        cancelled: Arc::new(AtomicBool::new(false)),
        supervisor: ProcessSupervisor::new(&PathBuf::from(env!("CARGO_BIN_EXE_ag1-process-host")))
            .unwrap(),
        rg: Some(PathBuf::from("/usr/bin/rg")),
        spools: dir.path().into(),
    };
    (dir, host)
}
#[test]
fn source_capsules_match_unchanged_sdk_required_forbidden_order_and_relationships() {
    let input = input();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/source-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["groups"].as_array().unwrap().len(), 8);
    for (scenario, expected) in input["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["groups"].as_array().unwrap())
    {
        let (_dir, host) = fixture(scenario);
        let options: RetrievalOptions = serde_json::from_value(scenario["input"].clone()).unwrap();
        let draft = build_capsule(&options, &host).unwrap();
        let record = &expected["record"];
        assert_eq!(draft.id, record["id"], "{} body", scenario["id"]);
        assert_eq!(
            json!(draft.files),
            record["files"],
            "{} files",
            scenario["id"]
        );
        assert_eq!(
            json!(draft.relations),
            record["relations"],
            "{} edges",
            scenario["id"]
        );
        assert_eq!(draft.retrieval, expected["result"]["details"]["retrieval"]);
        assert_eq!(draft.chars, record["chars"].as_u64().unwrap() as usize);
        assert_eq!(draft.truncated, record["truncated"].as_bool().unwrap());
        let text = expected["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            draft.body,
            text.split("\ncapsule_artifact:").next().unwrap()
        );
        for path in scenario["required"].as_array().unwrap() {
            assert!(draft.files.iter().any(|f| f.path == path.as_str().unwrap()));
        }
        for path in scenario["forbidden"].as_array().unwrap() {
            assert!(!draft.files.iter().any(|f| f.path == path.as_str().unwrap()));
        }
    }
}
#[test]
fn explicit_containment_cancel_and_missing_rg_cpu_fallback_remain_bounded() {
    let scenario = json!({});
    let (_dir, mut host) = fixture(&scenario);
    let options = RetrievalOptions {
        query: "TaskState".into(),
        paths: vec!["../outside".into()],
        ..Default::default()
    };
    assert!(build_capsule(&options, &host).is_err());
    host.cancelled
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(matches!(
        host.lexical(&["TaskState".into()]),
        Err(ag1_core::DomainError::Cancelled)
    ));
    host.cancelled
        .store(false, std::sync::atomic::Ordering::Release);
    host.rg = None;
    assert!(
        host.lexical(&["TaskState".into()])
            .unwrap()
            .contains_key("src/task.ts")
    );
}
#[test]
fn capsule_freshness_changes_without_rewriting_historical_slices() {
    let scenario = json!({});
    let (_dir, host) = fixture(&scenario);
    let options = RetrievalOptions {
        query: "ContextCompiler".into(),
        paths: vec!["src/compiler.ts".into()],
        ..Default::default()
    };
    let draft = build_capsule(&options, &host).unwrap();
    let original = draft.body.clone();
    fs::write(
        host.workspace.root().join("src/compiler.ts"),
        "export class ChangedCompiler {}\n",
    )
    .unwrap();
    let mut validator = SourceService::new(host.workspace.clone());
    let statuses: BTreeMap<_, _> = draft
        .files
        .iter()
        .map(|f| (f.reference.clone(), validator.validate(&f.reference).status))
        .collect();
    assert_eq!(
        ag1_core::context::retrieval::capsule_status(&draft.files, true, |r| statuses[r]),
        ag1_core::evidence::Freshness::Stale
    );
    assert_eq!(draft.body, original);
    assert_eq!(
        ag1_core::context::retrieval::capsule_status(&draft.files, false, |_| {
            ag1_core::evidence::Freshness::Current
        }),
        ag1_core::evidence::Freshness::Invalid
    );
}
