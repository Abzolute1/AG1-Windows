use std::{alloc::{GlobalAlloc,Layout,System},sync::atomic::{AtomicUsize,Ordering},time::Instant};
use ag1_core::{SessionId,context::{compiler::Compilation,features::Features}};
use ag1_storage::{Store,context::ContextStore};
use serde_json::json;
struct Counted;
static CURRENT:AtomicUsize=AtomicUsize::new(0);
static PEAK:AtomicUsize=AtomicUsize::new(0);
fn add(size:usize){let value=CURRENT.fetch_add(size,Ordering::Relaxed)+size;PEAK.fetch_max(value,Ordering::Relaxed);}
unsafe impl GlobalAlloc for Counted {
 unsafe fn alloc(&self,l:Layout)->*mut u8 {let p=unsafe{System.alloc(l)};if !p.is_null(){add(l.size());}p}
 unsafe fn alloc_zeroed(&self,l:Layout)->*mut u8 {let p=unsafe{System.alloc_zeroed(l)};if !p.is_null(){add(l.size());}p}
 unsafe fn dealloc(&self,p:*mut u8,l:Layout){unsafe{System.dealloc(p,l)};CURRENT.fetch_sub(l.size(),Ordering::Relaxed);}
 unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8{let q=unsafe{System.realloc(p,l,n)};if !q.is_null(){if n>=l.size(){add(n-l.size());}else{CURRENT.fetch_sub(l.size()-n,Ordering::Relaxed);}}q}
}
#[global_allocator]static ALLOCATOR:Counted=Counted;
fn main(){
 let label=std::env::args().nth(1).expect("variant label");
 let dir=tempfile::tempdir().unwrap();let store=Store::open(&dir.path().join("store")).unwrap();let now="2026-09-13T00:00:00Z";
 store.create_session("session",&serde_json::to_vec(&json!({"type":"session","version":3,"id":"session","cwd":"/fixture","timestamp":now})).unwrap()).unwrap();store.register_origin("session","session",&[7;32]).unwrap();
 let mut revision=0;
 for batch in 0..16 {
  let records=(batch*64..(batch+1)*64).map(|i|serde_json::to_vec(&json!({"type":"message","id":format!("m{i}"),"parentId":if i==0{None}else{Some(format!("m{}",i-1))},"timestamp":now,"message":{"role":"user","content":format!("{i}: {}","x".repeat(4096))}})).unwrap()).collect::<Vec<_>>();
  revision=store.append_batch("session",&format!("append-{batch}"),revision,&records).unwrap().revision;
 }
 let messages=vec![json!({"role":"user","content":"Measure exact custody; no external effects."})];let mut rows=Vec::new();
 for round in 0..4 {
  let mut context=ContextStore::new(&store,SessionId::new("session").unwrap(),"session",&format!("measure-{round}"),now,Features::default()).unwrap();
  let before=CURRENT.load(Ordering::Relaxed);PEAK.store(before,Ordering::Relaxed);let mut reduction_live=0;let time=Instant::now();
  let result=context.project(&messages,revision,|messages,_|{reduction_live=CURRENT.load(Ordering::Relaxed);Ok(Compilation{messages:messages.to_vec(),stats:json!({})})}).unwrap();
  let elapsed=time.elapsed().as_secs_f64()*1000.;let peak=PEAK.load(Ordering::Relaxed);let after=CURRENT.load(Ordering::Relaxed);
  assert_eq!(result.revision,revision);assert_eq!(result.projection.messages,messages);
  rows.push(json!({"round":round,"before_rust_bytes":before,"at_reduction_rust_bytes":reduction_live,"at_reduction_delta_bytes":reduction_live as i64-before as i64,"peak_delta_bytes":peak-before,"after_rust_bytes":after,"elapsed_ms":elapsed,"journal_artifact_bytes":result.original_journal.bytes}));
 }
 println!("{}",json!({"variant":label,"entries":1024,"content_bytes_per_entry":4096,"scope":"Actual native ContextStore::project; Rust global allocator requested bytes only, excludes SQLite C allocations, allocator overhead and model/runtime processes. Four rounds; first admits new custody blobs, subsequent rounds reuse identical blob bytes.","measurements":rows}));
}
