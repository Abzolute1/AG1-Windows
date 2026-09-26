use serde_json::{Value,json};
use ag1_core::runtime::session::Configuration;
use std::{alloc::{GlobalAlloc,Layout,System},sync::atomic::{AtomicBool,AtomicU64,Ordering},time::Instant,fs};
mod context {
 #[path="/home/alex/Projects/AG1/product/crates/ag1-platform/src/session/context/custody.rs"]
 mod custody;
 pub fn input(config:&ag1_core::runtime::session::Configuration,messages:&[serde_json::Value],revision:&Option<String>)->String { custody::input(config,messages,Some(revision)) }
}
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


fn main() {
 let args:Vec<_>=std::env::args().collect();
 let samples:Value=serde_json::from_slice(&fs::read(&args[1]).unwrap()).unwrap();
 let mut rows=vec![];
 for sample in samples.as_array().unwrap().iter().filter(|s|s["kind"]=="runtime-request-input") {
  let raw=fs::read(sample["path"].as_str().unwrap()).unwrap();
  let doc:Value=serde_json::from_slice(&raw).unwrap();
  let config=Configuration{model:doc["model"].clone(),tools:doc["tools"].as_array().unwrap().clone(),system_prompt:doc["systemPrompt"].as_str().unwrap().into(),thinking:String::new(),prompt:None,compiler:Default::default()};
  let messages=doc["messages"].as_array().unwrap();
  let revision:Option<String>=serde_json::from_value(doc["memoryRevision"].clone()).unwrap();
  for round in 0..3 {
   let (old,before)=measure(||json!({"version":1,"systemPrompt":config.system_prompt,"tools":config.tools,"model":config.model,"messages":messages,"memoryRevision":revision}).to_string());
   let (new,after)=measure(||context::input(&config,messages,&revision));
   assert_eq!(old.as_bytes(),raw);assert_eq!(new,old);
   rows.push(json!({"source_sha256":sample["sha256"],"bytes":raw.len(),"round":round,"before":before,"after":after,"exact_original_bytes":true}));
  }
 }
 fs::write(&args[2],serde_json::to_vec_pretty(&json!({"scope":"Actual old native JSON expression versus new production serializer on eight retained runtime-request-input snapshots, three pairs each. Serialization only; no model, storage, or context-compiler latency claim. Rust requested allocations only.","rows":rows})).unwrap()).unwrap();
}
