use serde_json::{Value,json};
use ag1_core::{SessionId,runtime::boundary::Boundary};
use std::{alloc::{GlobalAlloc,Layout,System},sync::atomic::{AtomicBool,AtomicU64,Ordering},time::Instant,fs};
#[path="/home/alex/Projects/AG1/implementation/evidence/overnight-final-followup-20260913/baseline-working-set.rs"] mod baseline;
#[path="/home/alex/Projects/AG1/product/crates/ag1-platform/src/session/working_set.rs"] mod candidate;
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
  let doc:Value=serde_json::from_slice(&fs::read(sample["path"].as_str().unwrap()).unwrap()).unwrap();
  let original=doc["messages"].as_array().unwrap();let prefix=original.len().saturating_sub(2);
  let branch:Vec<_>=original.iter().enumerate().map(|(i,m)|json!({"id":i,"parent":i.checked_sub(1),"type":"message","message":m})).collect();
  let key=json!({"model":doc["model"],"tools":doc["tools"],"system":doc["systemPrompt"]});
  let mut owner=Boundary::new(SessionId::new("fixture").unwrap(),1).unwrap();
  let compiled=original[prefix.saturating_sub(40)..prefix].to_vec();
  let old_identity=||baseline::RequestIdentity::new(&key,&compiled);
  let new_identity=||candidate::RequestIdentity::new(&key,&compiled);
  let old=baseline::WorkingSet::new(owner.stamp().clone(),key.clone(),branch[..prefix].to_vec(),original[..prefix].to_vec(),compiled.clone(),old_identity());
  let new=candidate::WorkingSet::new(owner.stamp().clone(),key.clone(),&branch[..prefix],&original[..prefix],compiled.clone(),new_identity());
  owner.changed().unwrap();
  for round in 0..3 {
   let old_branch=branch.clone();let old_original=original.clone();let old_request=old_identity();
   let (old_next,before)=measure(|| {
    let extended=old.extend(owner.stamp(),&key,&old_branch,&old_original).unwrap();
    let compiled=extended.clone();
    let result=baseline::WorkingSet::new(owner.stamp().clone(),key.clone(),old_branch,old_original,compiled,old_request);
    drop(extended);result
   });
   let new_branch=branch.clone();let new_original=original.clone();let new_request=new_identity();
   let (new_next,after)=measure(|| {
    let extended=new.extend(owner.stamp(),&key,&new_branch,&new_original).unwrap();
    // The compiler output is already owned in production. Move the same logical
    // sequence into finish without charging either side for compiler work.
    let compiled=extended.messages().to_vec();
    let result=extended.finish(compiled,new_request);
    drop(new_branch);drop(new_original);result
   });
   assert_eq!(old_next.compiled_len(),new_next.compiled_len());
   assert_eq!(old_next.extend(owner.stamp(),&key,&branch,original).unwrap(),new_next.extend(owner.stamp(),&key,&branch,original).unwrap().messages());
   rows.push(json!({"source_sha256":sample["sha256"],"messages":original.len(),"round":round,"before":before,"after":after,"identical_reuse":true}));
  }
 }
 fs::write(&args[2],serde_json::to_vec_pretty(&json!({"scope":"WorkingSet extension and next-cache construction using eight retained request histories, three pairs. Baseline and production candidate preserve identical future reuse. Both sides make the same compiler-output copy; compiler work itself is excluded. No live-model latency claim.","rows":rows})).unwrap()).unwrap();
}
