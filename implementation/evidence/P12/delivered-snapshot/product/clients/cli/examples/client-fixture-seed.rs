//! Creates a new private visual fixture. Never accepts an existing workspace.
use ag1_platform::client::{Config, Host};
use ag1_storage::Store;
use serde_json::json;
use std::path::PathBuf;
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(args.len(), 3);
    let workspace = PathBuf::from(&args[0]);
    let state = PathBuf::from(&args[1]);
    assert!(!workspace.exists() && !state.exists());
    let config: Config = serde_json::from_slice(&std::fs::read(&args[2]).unwrap()).unwrap();
    assert!(config.fixture);
    Host::init(&workspace).unwrap();
    let host = Host::open(&workspace, &state, config).await.unwrap();
    host.shutdown().await.unwrap();
    drop(host);
    let store = Store::open(&state.join("store")).unwrap();
    let mut records = vec![];
    for i in 0..1000 {
        records.push(serde_json::to_vec(&json!({"type":"message","id":format!("fixture-{i}"),"parentId":if i==0 {None}else{Some(format!("fixture-{}",i-1))},"timestamp":"2026-09-07T00:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":format!("Fixture message {i}: review the project and retain the exact source revision.")}],"timestamp":1788739200000u64}})).unwrap());
    }
    let submission=serde_json::from_value(json!({"summary":"Review an isolated project without altering the tuned model runtime.","steps":["Inspect the fixture file","Review the recorded diff"],"verification":["Compare exact source revisions"]})).unwrap();
    let plan = ag1_core::runtime::planning::Plan::from_submission(
        "visual-plan",
        "Review fixture changes",
        &submission,
        None,
        None,
        "2026-09-07T00:00:00.000Z",
    )
    .unwrap()
    .unwrap();
    records.push(serde_json::to_vec(&json!({"type":"custom","id":"fixture-plan","parentId":"fixture-999","timestamp":"2026-09-07T00:00:00.000Z","customType":ag1_core::runtime::planning::ARTIFACT,"data":plan})).unwrap());
    store
        .append_batch("initial", "visual-seed", 0, &records)
        .unwrap();
    println!("Seeded 1000 committed fixture messages");
}
