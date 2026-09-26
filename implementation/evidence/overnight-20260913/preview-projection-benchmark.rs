use ag1_platform::client::projection::message;
use ag1_protocol::ClientMessage;
use serde_json::{Value,json};
use std::{hint::black_box,time::Instant};
fn old(records:&[Value])->(usize,Vec<ClientMessage>) {
    let all:Vec<_>=records.iter().filter_map(message).collect();
    let mut result=Vec::new();let mut budget=8000;
    for m in all.iter().rev().take(100) {
        let n=serde_json::to_vec(m).unwrap().len();if n>budget {break;}budget-=n;result.push(m.clone());
    }
    result.reverse();(all.len(),result)
}
fn new(records:&[Value])->(usize,Vec<ClientMessage>) {
    let count=records.iter().filter(|r|r["type"]=="message").count();
    let mut result=Vec::new();let mut budget=8000;
    for m in records.iter().rev().filter_map(message).take(100) {
        let n=serde_json::to_vec(&m).unwrap().len();if n>budget {break;}budget-=n;result.push(m);
    }
    result.reverse();(count,result)
}
fn main() {
    let records:Vec<Value>=serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let mut rows=Vec::new();
    for scale in [1,4,16] {
        let input=records.iter().cycle().take(records.len()*scale).cloned().collect::<Vec<_>>();
        assert_eq!(serde_json::to_vec(&old(&input)).unwrap(),serde_json::to_vec(&new(&input)).unwrap());
        for order in [false,true,false,true] {
            for optimized in if order {[true,false]}else{[false,true]} {
                let start=Instant::now();let iterations=100;
                for _ in 0..iterations {black_box(if optimized {new(black_box(&input))}else{old(black_box(&input))});}
                rows.push(json!({"optimized":optimized,"records":input.len(),"iterations":iterations,"elapsed_ms":start.elapsed().as_secs_f64()*1000.}));
            }
        }
    }
    println!("{}",json!({"scope":"Preview projection only using a captured live branch and repeated synthetic scales; input construction excluded equally; no end-to-end claim","byte_equivalence":true,"rows":rows}));
}
