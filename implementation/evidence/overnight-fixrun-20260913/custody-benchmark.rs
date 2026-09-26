use ag1_storage::{Store,ArtifactInput,digest};
use serde_json::{Value,json};
use std::{alloc::{GlobalAlloc,Layout,System},sync::atomic::{AtomicBool,AtomicU64,Ordering},time::Instant,fs,path::Path};
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
 let args:Vec<String>=std::env::args().collect();
 let rows:Value=serde_json::from_slice(&fs::read(&args[1]).unwrap()).unwrap();
 let root=Path::new(&args[2]);fs::create_dir(root).unwrap();
 use std::os::unix::fs::PermissionsExt;fs::set_permissions(root,fs::Permissions::from_mode(0o700)).unwrap();
 let mut results=vec![];
 for mode in ["raw","compressed"] {
  let store=Store::open(&root.join(mode)).unwrap();
  store.create_session("fixture", &json!({"type":"session","version":3,"id":"fixture","cwd":"/fixture","timestamp":"2026-09-13T00:00:00Z"}).to_string().into_bytes()).unwrap();
  store.register_origin("fixture","fixture",&[7;32]).unwrap();
  for (i,row) in rows.as_array().unwrap().iter().enumerate() {
   let bytes=fs::read(row["path"].as_str().unwrap()).unwrap();assert_eq!(digest(&bytes),row["sha256"]);
   let id=format!("art_{i:032x}");
   let kind=if mode=="raw" { "text" } else { row["kind"].as_str().unwrap() };
   let record=json!({"version":1,"id":id,"sessionId":"fixture","kind":kind,"mimeType":"text/plain","encoding":"utf8","createdAt":"2026-09-13T00:00:00Z","sha256":digest(&bytes),"bytes":bytes.len(),"producer":{"type":"system","name":"sample"}});
   let input=ArtifactInput{origin:"fixture".into(),session:"fixture".into(),record:record.to_string().into_bytes(),bytes};
   let (_,publication)=measure(||store.put_artifact(&input).unwrap());
   let (page,read)=measure(||store.page("fixture","fixture",&id,0,32768).unwrap());
   assert_eq!(page.bytes,input.bytes[..page.bytes.len()]);
   results.push(json!({"mode":mode,"kind":row["kind"],"sha256":row["sha256"],"logical_bytes":input.bytes.len(),"publication":publication,"verified_page":read}));
  }
  drop(store);
 }
 let sizes:Vec<_>=["raw","compressed"].into_iter().map(|mode| {
  let dir=root.join(mode);let mut stack=vec![dir];let mut bytes=0u64;while let Some(dir)=stack.pop(){for entry in fs::read_dir(dir).unwrap(){let entry=entry.unwrap();if entry.file_type().unwrap().is_dir(){stack.push(entry.path())}else{bytes+=entry.metadata().unwrap().len();}}}
  json!({"mode":mode,"physical_file_bytes":bytes})
 }).collect();
 fs::write(&args[3],serde_json::to_vec_pretty(&json!({"scope":"One raw/compressed native publication and verified page per retained sample; 24 historical snapshots, CPU only. Rust requested allocation bytes exclude SQLite C/allocator overhead. Not a duration result.","rows":results,"stores":sizes})).unwrap()).unwrap();
}
