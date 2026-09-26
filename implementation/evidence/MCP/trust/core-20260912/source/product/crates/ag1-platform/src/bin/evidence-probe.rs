use ag1_core::{evidence::Freshness, task::TaskState};
use ag1_platform::{source::SourceService, workspace::WorkspaceService};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, time::Instant};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 4, "workspace state-json output-json");
    let registry = WorkspaceService::new().unwrap();
    let workspace = registry.register(Path::new(&args[1])).unwrap();
    let mut source = SourceService::new(workspace);
    let mut measurements = vec![];
    for size in [1024, 1024 * 1024, 8 * 1024 * 1024] {
        let name = format!("source-{size}.txt");
        let mut reads = vec![];
        let mut validation = vec![];
        let mut hashes = vec![];
        let mut digest = String::new();
        for _ in 0..5 {
            let start = Instant::now();
            let snapshot = source.capture(&name).unwrap();
            reads.push(start.elapsed().as_nanos() as u64);
            assert_eq!(snapshot.snapshot().bytes.len(), size);
            let start = Instant::now();
            digest = format!("{:x}", Sha256::digest(&snapshot.snapshot().bytes));
            hashes.push(start.elapsed().as_nanos() as u64);
            assert_eq!(snapshot.reference().digest(), digest);
            let start = Instant::now();
            assert_eq!(
                source.validate(&snapshot.reference().reference()).status,
                Freshness::Current
            );
            validation.push(start.elapsed().as_nanos() as u64);
        }
        measurements.push(json!({"bytes":size,"sha256":digest,"snapshot_ns":reads,"hash_ns":hashes,"repeated_validation_ns":validation}));
    }
    let state: Value = serde_json::from_slice(&fs::read(&args[2]).unwrap()).unwrap();
    let state = TaskState::parse(&state).unwrap();
    let mut serialization = vec![];
    let mut bytes = vec![];
    for _ in 0..100 {
        let start = Instant::now();
        bytes = serde_json::to_vec(state.value()).unwrap();
        serialization.push(start.elapsed().as_nanos() as u64);
    }
    let result = json!({"status":"measured","runtime":"rust","files":measurements,"state_bytes":bytes.len(),"state_sha256":format!("{:x}",Sha256::digest(&bytes)),"state_entries":state.value()["entries"].as_array().unwrap().len(),"serialization_ns":serialization,"model_actions":0,"network_actions":0,"scope":"Warm page-cache private files, no cache eviction. Snapshot reads and hashes the same full bytes with descriptor/path identity checks. Repeated validation caches parsed references only and rereads all bytes. Hash-only samples operate on resident bytes. TS cached validation uses metadata and provides a historical baseline, not equivalent freshness assurance."});
    fs::write(&args[3], serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    println!("measured three source sizes and 100 state serializations");
}
