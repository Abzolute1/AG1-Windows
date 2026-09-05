//! Deterministic in-memory provider/scheduler benchmark. No sockets or model processes.
use ag1_core::{
    OperationId,
    inference::{EndpointKind, Scheduler},
    provider::*,
};
use ag1_platform::provider::{WireDecoder, framing::SseDecoder};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Instant;
fn main() {
    let workload: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/providers/measurement-workload.json"
    ))
    .unwrap();
    let fragment = workload["fragment"].as_str().unwrap();
    let fragments = workload["fragments"].as_u64().unwrap() as usize;
    let frame = format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content":fragment},"finish_reason":null}]})
    );
    let bytes=(frame.repeat(fragments)+"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":10000}}\n\ndata: [DONE]\n\n").into_bytes();
    let mut samples = vec![];
    let mut output = String::new();
    for _ in 0..5 {
        let start = Instant::now();
        let mut decoder = WireDecoder::for_request(ApiFamily::Completions, "measurement").unwrap();
        let mut reducer = StreamReducer::new(ApiFamily::Completions, 0, 2 * 1024 * 1024).unwrap();
        let mut framing = SseDecoder::new(2 * 1024 * 1024).unwrap();
        for chunk in bytes.chunks(workload["transport_chunk_bytes"].as_u64().unwrap() as usize) {
            for frame in framing.feed(chunk).unwrap() {
                if frame.data == "[DONE]" {
                    continue;
                }
                for event in decoder.decode_json(frame.data.as_bytes()).unwrap() {
                    reducer.apply(event, 0).unwrap();
                }
            }
        }
        framing.finish().unwrap();
        let result = reducer.finish(false).unwrap();
        samples.push(start.elapsed().as_nanos() as u64);
        assert_eq!(result.stop_reason, Some(StopReason::Stop));
        output = result.content[0].text.clone();
        assert_eq!(output, fragment.repeat(fragments));
    }
    let mut queue_samples = vec![];
    let mut peak = 0;
    for batch in 0..workload["queue_batches"].as_u64().unwrap() {
        let scheduler = Scheduler::default();
        scheduler.register("fixture", EndpointKind::Local).unwrap();
        let start = Instant::now();
        let first = scheduler
            .submit(
                "fixture",
                OperationId::new(format!("{batch}-first")).unwrap(),
            )
            .unwrap();
        let permit = scheduler.start(&first).unwrap().unwrap();
        let tickets: Vec<_> = (0..workload["queue_waiters"].as_u64().unwrap())
            .map(|i| {
                scheduler
                    .submit("fixture", OperationId::new(format!("{batch}-{i}")).unwrap())
                    .unwrap()
            })
            .collect();
        peak = peak.max(scheduler.status("fixture").unwrap().waiting);
        permit.terminal().unwrap();
        for ticket in tickets {
            scheduler
                .start(&ticket)
                .unwrap()
                .unwrap()
                .terminal()
                .unwrap();
        }
        queue_samples.push(start.elapsed().as_nanos() as u64);
    }
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|line| {
                line.strip_prefix("VmHWM:")
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<u64>().ok())
            })
        });
    let result = json!({"status":"measured","runtime":"rust-domain-adapter","workload":workload,"wire_bytes":bytes.len(),"output_bytes":output.len(),"output_sha256":format!("{:x}",Sha256::digest(output.as_bytes())),"normalization_ns":samples,"queue_batch_ns":queue_samples,"peak_waiters":peak,"max_rss_kib":rss,"model_actions":0,"network_actions":0,"scope":"SSE framing and domain reduction, unlimited delivery in 97-byte chunks; excludes HTTP request setup. Queue measurements include operation identity checks and explicit admission. RSS includes only this fixture process. Not a desktop or live inference benchmark; TS setup/async overhead differs."});
    let path = std::env::args().nth(1).expect("measurement output path");
    std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    println!("measured {fragments} fragments and 100 bounded queue batches");
}
