//! Disposable-store measurement and abrupt-exit harness. Never controls model processes.
use ag1_core::Clock;
use ag1_storage::*;
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc, time::Instant};
struct ClockZero;
impl Clock for ClockZero {
    fn now_ms(&self) -> u64 {
        0
    }
}
struct ExitAt(Boundary);
impl FaultInjector for ExitAt {
    fn check(&self, b: Boundary) -> StorageResult<()> {
        if self.0 == b {
            std::process::exit(91);
        }
        Ok(())
    }
}
fn header() -> Vec<u8> {
    serde_json::to_vec(
        &json!({"type":"session","version":3,"id":"s","timestamp":"t","cwd":"/frozen"}),
    )
    .unwrap()
}
fn artifact(id: usize) -> ArtifactInput {
    let bytes = format!("artifact {id}\nneedle evidence\n{}\n", "x".repeat(4000)).into_bytes();
    let record = json!({"version":1,"id":format!("art_{id:032x}"),"sessionId":"s","kind":"text","encoding":"utf8","mimeType":"text/plain","createdAt":"2026-09-05T00:00:00Z","producer":{"type":"system","name":"probe"},"bytes":bytes.len(),"sha256":digest(&bytes)});
    ArtifactInput {
        origin: "s".into(),
        session: "s".into(),
        record: serde_json::to_vec(&record).unwrap(),
        bytes,
    }
}
fn seed(path: &Path) -> StorageResult<()> {
    let s = Store::open(path)?;
    s.create_session("s", &header())?;
    s.register_origin("s", "s", &[9; 32])
}
fn crash(path: &Path, point: Boundary, operation: &str) -> StorageResult<()> {
    let s = Store::open_with(path, Arc::new(ClockZero), Arc::new(ExitAt(point)))?;
    match operation {
        "artifact" => {
            s.put_artifact(&artifact(1))?;
        }
        "intent" => {
            s.record_intent("external", b"external effect intent")?;
        }
        "journal" => {
            s.append_batch("s","append",0,&[serde_json::to_vec(&json!({"type":"custom","customType":"probe","id":"e","parentId":null,"timestamp":"t"}))?])?;
        }
        _ => return Err(StorageError::Invalid("Crash operation".into())),
    }
    if point == Boundary::AfterAck {
        fs::write(path.join("delivered-ack"), b"accepted durable API return\n")?;
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.join("delivered-ack"))?
            .sync_all()?;
        std::process::exit(91);
    }
    Err(StorageError::Invalid(
        "Crash boundary was not reached".into(),
    ))
}
fn ns(start: Instant) -> u64 {
    start.elapsed().as_nanos().max(1) as u64
}
fn measure(output: &Path) -> StorageResult<()> {
    let root = tempfile::tempdir()?;
    let mut rows = Vec::new();
    for count in [10_000, 100_000] {
        let source = root.path().join(format!("source-{count}"));
        fs::create_dir(&source)?;
        let mut bytes = header();
        bytes.push(b'\n');
        for i in 0..count {
            let v = json!({"type":"custom","customType":"benchmark","id":format!("e{i}"),"parentId":if i==0{Value::Null}else{format!("e{}",i-1).into()},"timestamp":"2026-09-05T00:00:00Z","data":{"evidence":"retained journal bytes"}});
            bytes.extend(serde_json::to_vec(&v)?);
            bytes.push(b'\n');
        }
        fs::write(source.join("s.jsonl"), &bytes)?;
        let manifest = FrozenManifest {
            version: 1,
            frozen: true,
            files: vec![FrozenFile {
                path: "s.jsonl".into(),
                sha256: digest(&bytes),
                bytes: bytes.len() as u64,
                kind: FileKind::Journal,
                session: Some("s".into()),
                origin: None,
            }],
        };
        let path = root.path().join(format!("store-{count}"));
        let store = Store::open(&path)?;
        let start = Instant::now();
        store.import_frozen(&source, &manifest)?;
        let import_ns = ns(start);
        drop(store);
        let start = Instant::now();
        let store = Store::open(&path)?;
        let snapshot = store.snapshot("s")?;
        let resume_ns = ns(start);
        assert_eq!(snapshot.entries.len(), count);
        let start = Instant::now();
        let branch = store.branch("s", None)?;
        let branch_ns = ns(start);
        assert_eq!(branch.len(), count);
        rows.push(json!({"entries":count,"source_bytes":bytes.len(),"import_ns":import_ns,"resume_ns":resume_ns,"branch_ns":branch_ns,"resumed_entries":snapshot.entries.len()}));
    }
    let path = root.path().join("artifact-store");
    seed(&path)?;
    let store = Store::open(&path)?;
    let mut ack = Vec::new();
    for i in 0..100 {
        let start = Instant::now();
        store.append_batch("s",&format!("c{i}"),i,&[serde_json::to_vec(&json!({"type":"custom","customType":"ack","id":format!("e{i}"),"parentId":if i==0{Value::Null}else{format!("e{}",i-1).into()},"timestamp":"t"}))?])?;
        ack.push(ns(start));
    }
    let mut catalogs = Vec::new();
    let mut inserted = 0;
    for size in [10, 100, 1000] {
        let start = Instant::now();
        for i in inserted..size {
            store.put_artifact(&artifact(i))?;
        }
        let growth_ns = ns(start);
        inserted = size;
        let start = Instant::now();
        let hits = store.search("s", "needle", 100)?;
        let warm_search_ns = ns(start);
        assert_eq!(hits.len(), size.min(100));
        let start = Instant::now();
        let page = store.page("s", "s", &format!("art_{:032x}", size - 1), 0, 32768)?;
        let warm_page_ns = ns(start);
        assert!(page.bytes.starts_with(b"artifact"));
        catalogs.push(json!({"records":size,"growth_ns":growth_ns,"warm_search_ns":warm_search_ns,"warm_page_ns":warm_page_ns}));
    }
    drop(store);
    let start = Instant::now();
    let store = Store::open(&path)?;
    let reopen_ns = ns(start);
    let start = Instant::now();
    assert_eq!(store.search("s", "needle", 100)?.len(), 100);
    let cold_search_ns = ns(start);
    let start = Instant::now();
    store.page("s", "s", &format!("art_{:032x}", 999), 0, 32768)?;
    let cold_page_ns = ns(start);
    ack.sort_unstable();
    let report = json!({"status":"measured","platform":std::env::consts::OS,"sqlite":rusqlite::version(),"durability":"WAL synchronous=FULL; blobs and directories synced before reference commit","journal":rows,"durable_ack":{"samples_ns":ack,"p50_ns":ack[49],"p95_ns":ack[94]},"catalogs":catalogs,"cold":{"reopen_ns":reopen_ns,"search_ns":cold_search_ns,"page_ns":cold_page_ns},"notes":["Cold means reopened adapter, not flushed kernel cache; no system cache eviction performed.","Reference TS append is unsynced, so no equivalent-durability speed ratio is claimed.","Single sequential Linux run on disposable stores; no live TS writes or model actions.","Resume includes integrity verification and full journal materialization; artifact pages verify the blob hash."],"model_actions":0});
    fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"status":"measured","journal_sizes":[10000,100000],"catalog_sizes":[10,100,1000],"ack_samples":100})
    );
    Ok(())
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("seed") if args.len() == 3 => seed(Path::new(&args[2])),
        Some("crash") if args.len() == 5 => {
            serde_json::from_str::<Boundary>(&format!("\"{}\"", args[3]))
                .map_err(StorageError::Json)
                .and_then(|b| crash(Path::new(&args[2]), b, &args[4]))
        }
        Some("measure") if args.len() == 3 => measure(Path::new(&args[2])),
        Some("export-fixture") if args.len() == 3 => export_fixture(Path::new(&args[2])),
        _ => Err(StorageError::Invalid(
            "Usage: storage-probe seed PATH | crash PATH BOUNDARY OP | measure OUTPUT".into(),
        )),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn export_fixture(path: &Path) -> StorageResult<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("s");
    seed(&source)?;
    let s = Store::open_with(
        &source,
        Arc::new(ClockZero),
        Arc::new(ExitAt(Boundary::AfterAck)),
    )?;
    s.append_batch("s","command",0,&[serde_json::to_vec(&json!({"type":"custom","customType":"probe","id":"entry","parentId":null,"timestamp":"2026-09-05T00:00:00Z","data":{"unknown":"preserved"}}))?])?;
    let id = s.put_artifact(&artifact(7))?;
    s.lease("s", "s", &id, "verify export", 60000, true)?;
    let cursor = s.cursor("s", "s", &id, 1, 5)?;
    s.export(path, true)?;
    println!(
        "{}",
        json!({"artifact":id,"cursor":cursor,"entries":1,"leases":1})
    );
    Ok(())
}
