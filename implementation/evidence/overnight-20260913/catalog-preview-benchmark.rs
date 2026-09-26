use ag1_storage::{ArtifactInput, Store, digest};
use serde_json::{Value,json};
use std::{hint::black_box,time::Instant};
fn main() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(&dir.path().join("store")).unwrap();
    store.create_session("fixture",&serde_json::to_vec(&json!({"type":"session","version":3,"id":"fixture","cwd":"/fixture","timestamp":"2026-09-13T00:00:00Z"})).unwrap()).unwrap();
    store.register_origin("origin","fixture",&[7;32]).unwrap();
    let mut last=None;
    for i in 0..2048 {
        let bytes=format!("retained source {i}").into_bytes();
        let record=json!({"version":1,"id":format!("art_{i:032x}"),"sessionId":"fixture","kind":if i%16==0 {"text"}else{"runtime-request-input"},"mimeType":"text/plain","encoding":"utf8","createdAt":"2026-09-13T00:00:00Z","sha256":digest(&bytes),"bytes":bytes.len(),"producer":{"type":"system","name":"fixture"}});
        let input=ArtifactInput{origin:"origin".into(),session:"fixture".into(),record:serde_json::to_vec(&record).unwrap(),bytes};
        store.put_artifact(&input).unwrap();last=Some(input);
    }
    let expected=store.evidence_catalog("fixture",20).unwrap().into_iter().map(|r|r.record).collect::<Vec<Value>>();
    let mut rows=vec![];
    for round in 0..32 {
        // Re-admission of the exact immutable artifact changes the native store
        // sequence, invalidating its cache without changing the returned catalog.
        // That write is deliberately outside the measured query intervals.
        store.put_artifact(last.as_ref().unwrap()).unwrap();
        let start=Instant::now();let cold=store.evidence_catalog("fixture",20).unwrap();let cold_ms=start.elapsed().as_secs_f64()*1000.;
        assert_eq!(cold.into_iter().map(|r|r.record).collect::<Vec<_>>(),expected);
        let start=Instant::now();
        for _ in 0..100 {black_box(store.evidence_catalog("fixture",20).unwrap());}
        let warm_ms=start.elapsed().as_secs_f64()*1000./100.;
        assert_eq!(store.evidence_catalog("fixture",20).unwrap().into_iter().map(|r|r.record).collect::<Vec<_>>(),expected);
        rows.push(json!({"round":round,"cold_ms":cold_ms,"warm_ms":warm_ms}));
    }
    println!("{}",json!({"scope":"Actual native Store evidence_catalog cold-cache versus warm-cache query, 2048 immutable artifacts (1/16 ordinary evidence), setup and invalidating writes excluded; shared workstation, not whole-app CPU saving","returned_records":expected.len(),"identical_results":true,"rows":rows}));
}
