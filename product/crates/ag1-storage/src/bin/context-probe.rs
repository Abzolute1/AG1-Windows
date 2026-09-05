//! Offline P08 measurement driver. Global allocator instrumentation is confined to this binary.
#[path = "../../../ag1-core/tests/support/compiler_fixture.rs"]
mod fixture;
use ag1_core::{
    SessionId,
    context::{
        compiler::{CompileOptions, ContextSnapshot, compile_snapshot},
        features::Features,
    },
};
use ag1_storage::{Store, context::ContextStore, digest};
use serde_json::{Value, json};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs,
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};
struct Meter;
static METERED: AtomicBool = AtomicBool::new(true);
static ENABLED: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
// SAFETY: every allocation/deallocation delegates unchanged to System; counters never own pointers.
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && METERED.load(Ordering::Relaxed) {
            let live =
                LIVE.fetch_add(layout.size() as u64, Ordering::Relaxed) + layout.size() as u64;
            if ENABLED.load(Ordering::Relaxed) {
                CALLS.fetch_add(1, Ordering::Relaxed);
                BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
                PEAK.fetch_max(live, Ordering::Relaxed);
            }
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if METERED.load(Ordering::Relaxed) {
            LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() && METERED.load(Ordering::Relaxed) {
            let old = LIVE.fetch_add(size as u64, Ordering::Relaxed);
            if METERED.load(Ordering::Relaxed) {
                LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
            }
            if ENABLED.load(Ordering::Relaxed) {
                CALLS.fetch_add(1, Ordering::Relaxed);
                BYTES.fetch_add(size as u64, Ordering::Relaxed);
                PEAK.fetch_max(old + size as u64 - layout.size() as u64, Ordering::Relaxed);
            }
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: Meter = Meter;
fn measure<T>(run: impl FnOnce() -> T) -> (T, Value) {
    let live = LIVE.load(Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    PEAK.store(live, Ordering::Relaxed);
    ENABLED.store(true, Ordering::Relaxed);
    let start = Instant::now();
    let value = run();
    let nanos = start.elapsed().as_nanos() as u64;
    ENABLED.store(false, Ordering::Relaxed);
    (
        value,
        if METERED.load(Ordering::Relaxed) {
            json!({"ns":nanos,"allocation_calls":CALLS.load(Ordering::Relaxed),"requested_allocation_bytes":BYTES.load(Ordering::Relaxed),"peak_extra_live_bytes":PEAK.load(Ordering::Relaxed).saturating_sub(live)})
        } else {
            json!({"ns":nanos})
        },
    )
}
fn record_hash(value: &Value) -> String {
    digest(value.to_string().as_bytes())
}
fn durable(root: &Path, messages: &[Value], options: &CompileOptions) -> Value {
    let store = Store::open(root).unwrap();
    store.create_session("fixture",&json!({"type":"session","version":3,"id":"fixture","cwd":"/fixture","timestamp":"2026-09-05T00:00:00.000Z"}).to_string().into_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[7; 32])
        .unwrap();
    let mut context = ContextStore::new(
        &store,
        SessionId::new("fixture").unwrap(),
        "fixture",
        "compile",
        "2026-09-05T00:00:00.000Z",
        Features::default(),
    )
    .unwrap();
    let (cold, cold_measure) = measure(|| context.compile(messages, options, 0).unwrap());
    let (_, warm_measure) = measure(|| {
        context
            .compile(&cold.projection.messages, options, 0)
            .unwrap()
    });
    let id = cold.original_request.id.clone();
    let sha = cold.original_request.sha256.clone();
    drop(context);
    drop(store);
    let (store, reopen_measure) = measure(|| Store::open(root).unwrap());
    let (_, read_measure) = measure(|| {
        let mut bytes = vec![];
        let mut offset = 0;
        loop {
            let page = store
                .page("fixture", "fixture", &id, offset, 32768)
                .unwrap();
            bytes.extend(page.bytes);
            match page.next_offset {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(digest(&bytes), sha);
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!(messages)
        );
    });
    let mut logical = 0;
    let mut files = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                stack.push(e.path());
            } else {
                files += 1;
                logical += e.metadata().unwrap().len();
            }
        }
    }
    json!({"cold":cold_measure,"warm":warm_measure,"reopen":reopen_measure,"rehydrate_original":read_measure,"original_bytes":cold.original_request.bytes,"original_sha256":sha,"physical_files":files,"logical_file_bytes":logical,"stats":cold.projection.stats})
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        (3..=4).contains(&args.len()),
        "private-root output-json [--timing]"
    );
    if args.get(3).is_some_and(|v| v == "--timing") {
        METERED.store(false, Ordering::Relaxed);
    }
    let root = Path::new(&args[1]);
    fs::create_dir_all(root).unwrap();
    let recipes: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/context/compiler-inputs.json"
    ))
    .unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/context/compiler-oracle.json"
    ))
    .unwrap();
    let mut groups = vec![];
    for recipe in recipes["cases"].as_array().unwrap().iter().filter(|r| {
        ["32k-pressure", "protected-201", "externalized-2m-tokens"]
            .contains(&r["id"].as_str().unwrap())
    }) {
        let id = recipe["id"].as_str().unwrap();
        let (messages, options) =
            fixture::recipe_input(recipe, &mut fixture::FixtureStore::default());
        let expected = oracle["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == id)
            .unwrap();
        let mut cold_samples = vec![];
        let mut warm_samples = vec![];
        let mut bytes = 0;
        let mut records = 0;
        let mut rehydrate = vec![];
        for _ in 0..25 {
            let mut store = fixture::FixtureStore::default();
            let input = messages.clone();
            let (cold, cold_sample) = measure(|| {
                let captured = ContextSnapshot::capture(input).unwrap();
                compile_snapshot(&captured, &options, &mut store).unwrap()
            });
            let (warm, warm_sample) =
                measure(|| compile_snapshot(&cold.snapshot, &options, &mut store).unwrap());
            assert_eq!(cold.stats, expected["cold"]["stats"]);
            assert_eq!(warm.stats, expected["warm"]["stats"]);
            assert_eq!(
                json!(cold.snapshot.values().collect::<Vec<_>>()),
                expected["cold"]["messages"]
            );
            assert_eq!(
                json!(warm.snapshot.values().collect::<Vec<_>>()),
                expected["warm"]["messages"]
            );
            cold_samples.push(cold_sample);
            warm_samples.push(warm_sample);
            bytes = store.texts.values().map(|s| s.len()).sum::<usize>();
            records = store.records.len();
            let (_, sample) = measure(|| {
                for r in &store.records {
                    let text = &store.texts[r["id"].as_str().unwrap()];
                    assert_eq!(digest(text.as_bytes()), r["sha256"]);
                }
            });
            rehydrate.push(sample);
        }
        let (_, validation) = measure(|| ag1_core::context::validate_history(&messages).unwrap());
        let (_, estimates) = measure(|| {
            messages
                .iter()
                .map(ag1_core::context::estimate_tokens)
                .sum::<u64>()
        });
        let (_, salience) = measure(|| {
            messages
                .iter()
                .filter_map(|m| m["content"][0]["text"].as_str())
                .map(ag1_core::context::salient::select)
                .collect::<Vec<_>>()
        });
        let (_, normalization) = measure(|| {
            messages
                .iter()
                .filter_map(|m| m["content"][0]["text"].as_str())
                .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
        });
        let components = json!({"validation":validation,"estimates":estimates,"salience":salience,"normalization":normalization});
        let durable_dir = tempfile::tempdir_in(root).unwrap();
        let durable = durable(&durable_dir.path().join("store"), &messages, &options);
        groups.push(json!({"id":id,"input_messages":messages.len(),"input_bytes":serde_json::to_vec(&messages).unwrap().len(),"input_sha256":record_hash(&json!(messages)),"cold":cold_samples,"warm":warm_samples,"rehydration":rehydrate,"artifact_bytes":bytes,"artifacts":records,"cold_stats":expected["cold"]["stats"],"warm_stats":expected["warm"]["stats"],"cold_sha256":record_hash(&expected["cold"]["messages"]),"warm_sha256":record_hash(&expected["warm"]["messages"]),"durable":durable,"component_diagnostics":components}));
    }
    let out = json!({"status":"measured","runtime":"rust","scope":"Owned snapshot capture plus volatile callbacks isolate compiler behavior. Input ownership is transferred, without cloning inside the measured window; immutable snapshot validation and token estimates are included in cold, reused in warm. Artifact metadata and leases are refreshed every pass; no durability claim for those samples. Separate P05 rows include WAL FULL, blob/directory sync, original journal/request custody, integrity-checked reopen and page rehydration. Kernel caches are not evicted. Allocation values are allocator requests, not RSS. Logical file bytes are retained sizes, not physical device writes.","allocator_instrumentation":METERED.load(Ordering::Relaxed),"samples":25,"groups":groups,"model_actions":0,"network_actions":0});
    fs::write(&args[2], serde_json::to_vec_pretty(&out).unwrap()).unwrap();
    println!(
        "measured three compiler histories, cold/warm allocations, durable custody and rehydration"
    );
}
