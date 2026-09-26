use ag1_protocol::{self as w, ClientView};
use std::{hint::black_box, time::Instant};
fn old(view: ClientView, last: &mut String) -> Option<Vec<u8>> {
    let serialized = serde_json::to_string(&view).unwrap();
    if serialized == *last { return None; }
    *last = serialized;
    Some(w::encode(&w::WireMessage::Client(Box::new(view.clone()))).unwrap())
}
fn new(view: ClientView, last: &mut Vec<u8>) -> bool {
    let encoded = w::encode(&w::WireMessage::Client(Box::new(view))).unwrap();
    if encoded == *last { return false; }
    *last = encoded;
    true
}
fn main() {
    let bytes = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let w::WireMessage::Client(base) = w::decode(&bytes).unwrap() else { panic!("client fixture") };
    let mut old_last = String::new(); let mut new_last = Vec::new();
    for step in [0,0,1,1,2,3,3,0] {
        let mut view = (*base).clone(); view.progress = format!("stream {step}");
        let expected = old(view.clone(), &mut old_last);
        let changed = new(view, &mut new_last);
        assert_eq!(expected.is_some(), changed);
        if let Some(expected) = expected { assert_eq!(expected, new_last); }
    }
    let iterations = 10000;
    let mut rows = vec![];
    for order in [false, true, false, true] {
        for optimized in if order { [true,false] } else { [false,true] } {
            for changed in [true, false] {
                let mut old_last = String::new(); let mut new_last = Vec::new();
                let started = Instant::now(); let mut emitted = 0;
                for i in 0..iterations {
                    let mut view = (*base).clone();
                    view.progress = format!("stream {}", if changed { i } else { 0 });
                    if optimized {
                        if new(view, &mut new_last) { emitted += 1; black_box(&new_last); }
                    } else if let Some(bytes) = old(view, &mut old_last) { emitted += 1; black_box(bytes); }
                }
                rows.push(serde_json::json!({"optimized":optimized,"changed_each_tick":changed,"iterations":iterations,"emitted":emitted,"elapsed_ms":started.elapsed().as_secs_f64()*1000.}));
            }
        }
    }
    println!("{}", serde_json::json!({"byte_and_dedup_equivalence":true,"input_frame_bytes":bytes.len(),"scope":"Serialization microbenchmark using actual CLI view; common fixture clone included in both paths; no transport/I/O or end-to-end speedup claim","rows":rows}));
}
