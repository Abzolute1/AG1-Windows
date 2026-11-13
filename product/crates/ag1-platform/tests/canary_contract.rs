#![cfg(target_os = "linux")]

#[path = "../../../../fixtures/runtime/custody.rs"]
mod fixture;
use ag1_core::PortFuture;
use ag1_platform::{
    cutover::{
        Registry,
        canary::{
            self, Answer, Driver, Request, Runner,
            native::{Reference, TransportDriver},
        },
    },
    llama::{Material, custody::RuntimeBundle},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
fn models() -> Vec<Value> {
    ["one", "two", "three"]
        .into_iter()
        .map(|id| {
            json!({"id":id,"name":id,"provider":"llamacpp",
        "api":"openai-completions","baseUrl":"http://127.0.0.1:18087/v1","reasoning":true,
        "input":["text"],"contextWindow":32768,"maxTokens":4096,
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}})
        })
        .collect()
}
fn message(case: &str) -> Value {
    let mut message = json!({"role":"assistant","content":[],"stopReason":"stop"});
    match case {
        "exact-copy" => {
            message["content"] = json!([{"type":"text","text":"P13_COPY_7e2b: αβ 0192837465"}])
        }
        "reasoning" => {
            message["content"] =
                json!([{"type":"thinking","thinking":"37 * 20 - 37"},{"type":"text","text":"703"}])
        }
        "tool-call" => {
            message["stopReason"] = json!("toolUse");
            message["content"] = json!([{"type":"toolCall","id":"p13-call","name":"p13_echo","arguments":{"value":"P13_TOOL_7"}}]);
        }
        "tool-result" => message["content"] = json!([{"type":"text","text":"P13_TOOL_7"}]),
        "context-limit" => {
            message["stopReason"] = json!("error");
            message["errorMessage"] = json!("400: request exceeds model context size");
        }
        "cancellation" => {
            message["stopReason"] = json!("aborted");
            message["content"] = json!([{"type":"text","text":"1\n"}]);
        }
        _ => panic!("unexpected case"),
    }
    message
}
#[derive(Default)]
struct Fake {
    calls: Vec<(String, Runner, String)>,
    resets: usize,
    bad: bool,
}
impl Driver for Fake {
    fn reset(&mut self) -> PortFuture<'_, ()> {
        self.resets += 1;
        Box::pin(async { Ok(()) })
    }
    fn run<'a>(&'a mut self, runner: Runner, r: &'a Request) -> PortFuture<'a, Answer> {
        Box::pin(async move {
            self.calls.push((
                r.model["id"].as_str().unwrap().into(),
                runner,
                r.case.clone(),
            ));
            let mut answer = Answer {
                message: message(&r.case),
                payload: canary::payload(r)?,
                first_content_ms: Some(1),
                elapsed_ms: 2,
                cancelled: r.cancel_after_first_content,
                settled: true,
                runtime_before: None,
                runtime_after: None,
            };
            if self.bad && r.case == "reasoning" {
                answer.message["content"] = json!([{"type":"text","text":"702"}]);
            }
            Ok(answer)
        })
    }
}
#[tokio::test]
async fn fixture_matrix_requires_every_case_for_each_model_and_never_claims_live_acceptance() {
    let mut fake = Fake::default();
    let report = canary::rehearse(&mut fake, &models()).await;
    assert_eq!(report.status, "passed");
    assert!(report.fixture);
    assert_eq!(fake.calls.len(), 36);
    assert_eq!(fake.resets, 7);
    for (index, model) in ["one", "two", "three"].iter().enumerate() {
        assert!(
            fake.calls[index * 12..(index + 1) * 12]
                .iter()
                .all(|c| &c.0 == model)
        );
        assert!(
            fake.calls[index * 12..index * 12 + 6]
                .iter()
                .all(|c| c.1 == Runner::Typescript)
        );
        assert!(
            fake.calls[index * 12 + 6..(index + 1) * 12]
                .iter()
                .all(|c| c.1 == Runner::Rust)
        );
    }
    let mut duplicate = models();
    duplicate[1] = duplicate[0].clone();
    assert_eq!(
        canary::rehearse(&mut Fake::default(), &duplicate)
            .await
            .status,
        "failed"
    );
    assert_eq!(
        canary::rehearse(&mut Fake::default(), &models()[..2])
            .await
            .status,
        "failed"
    );
}
#[tokio::test]
async fn failed_quality_retains_the_failed_answer_and_settles_without_trying_other_models() {
    let mut fake = Fake {
        bad: true,
        ..Fake::default()
    };
    let report = canary::rehearse(&mut fake, &models()).await;
    assert_eq!(report.status, "failed");
    assert!(report.runtime_settled);
    assert_eq!(report.rows.len(), 2);
    assert_eq!(fake.resets, 2);
    assert_eq!(report.rows[1].answer.message["content"][0]["text"], "702");
}

#[tokio::test]
async fn recorded_matrix_rechecks_quality_payload_context_order_and_tool_causality() {
    let catalog = models();
    let report = canary::rehearse(&mut Fake::default(), &catalog).await;
    canary::verify_recording(&report, &catalog).unwrap();
    for mutation in 0..10 {
        let mut changed = report.clone();
        match mutation {
            0 => changed.rows[0].answer.message["content"][0]["text"] = "wrong exact copy".into(),
            1 => changed.rows[1].answer.message["content"][1]["text"] = "702".into(),
            2 => changed.rows[2].answer.message["content"][0]["id"] = "different-tool-call".into(),
            3 => changed.rows[3].answer.message["content"][0]["text"] = "wrong result".into(),
            4 => changed.rows[4].answer.message["errorMessage"] = "network error".into(),
            5 => changed.rows[5].answer.cancelled = false,
            6 => changed.rows[0].payload_sha256 = "0".repeat(64),
            7 => changed.rows[0].answer.first_content_ms = Some(3),
            8 => changed.rows.swap(0, 6),
            _ => changed.rows.pop().map(|_| ()).unwrap(),
        }
        assert!(
            canary::verify_recording(&changed, &catalog).is_err(),
            "mutation {mutation}"
        );
    }
    let mut changed_catalog = catalog.clone();
    changed_catalog[0]["contextWindow"] = 65536.into();
    assert!(canary::verify_recording(&report, &changed_catalog).is_err());
    let mut changed = report.clone();
    changed.rows[0].answer.payload["temperature"] = json!(0.123);
    changed.rows[0].payload_sha256 =
        ag1_storage::digest(&serde_json::to_vec(&changed.rows[0].answer.payload).unwrap());
    assert!(canary::verify_recording(&changed, &catalog).is_err());
}

#[tokio::test]
async fn recorded_live_shape_requires_consistent_resource_samples_and_never_grants_authority() {
    let catalog = models();
    let mut report = canary::rehearse(&mut Fake::default(), &catalog).await;
    // Synthetic validator input only; this cannot construct Acceptance.
    report.fixture = false;
    assert!(canary::verify_recording(&report, &catalog).is_err());
    for (i, row) in report.rows.iter_mut().enumerate() {
        let sample = json!({"owned_pid":100+i/6,"resident_kib":1024,
            "gpu":{"total_mib":16384,"used_mib":15000,"free_mib":1384}});
        row.answer.runtime_before = Some(sample.clone());
        if row.case != "cancellation" {
            row.answer.runtime_after = Some(sample);
        }
    }
    canary::verify_recording(&report, &catalog).unwrap();
    for mutation in 0..4 {
        let mut changed = report.clone();
        match mutation {
            0 => changed.rows[1].answer.runtime_before.as_mut().unwrap()["owned_pid"] = 999.into(),
            1 => changed.rows[0].answer.runtime_after.as_mut().unwrap()["owned_pid"] = 999.into(),
            2 => changed.rows[0].answer.runtime_before.as_mut().unwrap()["resident_kib"] = 0.into(),
            _ => {
                changed.rows[0].answer.runtime_before.as_mut().unwrap()["gpu"]["free_mib"] =
                    20000.into()
            }
        }
        assert!(canary::verify_recording(&changed, &catalog).is_err());
    }
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("synthetic-verifier-input.json");
    fs::write(
        &file,
        serde_json::to_vec(&json!({"report":report,"models":catalog})).unwrap(),
    )
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let result = std::process::Command::new(root.join("product/target/debug/ag1-cutover"))
        .arg("verify-canaries")
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["status"], "verified");
    assert_eq!(value["runtime_control_granted"], false);
    assert_eq!(value["authority_transferred"], false);
    fs::write(&file, b"{}").unwrap();
    assert!(canary::verify_recording_file(&file).is_err());
    let link = temp.path().join("linked.json");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(canary::verify_recording_file(&link).is_err());
}
fn material(path: &Path) -> Material {
    use sha2::{Digest, Sha256};
    Material {
        path: path.display().to_string(),
        sha256: format!("{:x}", Sha256::digest(fs::read(path).unwrap())),
        kind: "fixture-helper".into(),
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_pinned_ts_and_native_transports_rehearse_all_models_with_owned_cancellation() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicBool, Ordering},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    server.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let stop = done.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let audit = Arc::new(Mutex::new(fixture::Audit::default()));
    let server_audit = audit.clone();
    let thread = std::thread::spawn(move || {
        'connections: while !stop.load(Ordering::SeqCst) {
            let Ok((mut socket, _)) = server.accept() else {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = vec![];
            let mut block = [0; 8192];
            let header_end = loop {
                let n = socket.read(&mut block).unwrap();
                if n == 0 {
                    continue 'connections;
                }
                bytes.extend_from_slice(&block[..n]);
                if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let header = String::from_utf8_lossy(&bytes[..header_end]);
            let len: usize = header
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < header_end + len {
                let n = socket.read(&mut block).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&block[..n]);
            }
            let input: Value =
                serde_json::from_slice(&bytes[header_end..header_end + len]).unwrap();
            assert_eq!(
                server_audit.lock().unwrap().active.as_deref(),
                input["model"].as_str()
            );
            recorded.lock().unwrap().push(input.clone());
            let messages = input["messages"].as_array().unwrap();
            assert!(messages.iter().any(|message| {
                matches!(message["role"].as_str(), Some("system" | "developer"))
                    && message["content"].as_str().is_some_and(|text| {
                        text.starts_with("You are helping verify a local agent migration.")
                            && text.contains("the requested value is the whole response")
                    })
            }));
            let prompt = messages.iter().find(|m| m["role"] == "user").unwrap()["content"]
                .as_str()
                .unwrap();
            if prompt.len() > 60000 {
                let body = r#"{"error":{"message":"request exceeds model context size"}}"#;
                write!(socket,"HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                continue;
            }
            let case = if prompt.starts_with("Count upward") {
                "cancellation"
            } else if prompt.contains("37 times") {
                "reasoning"
            } else if messages.last().unwrap()["role"] == "tool" {
                "tool-result"
            } else if prompt.starts_with("Call p13_echo") {
                "tool-call"
            } else {
                "exact-copy"
            };
            assert_eq!(
                input["max_tokens"],
                if case == "reasoning" {
                    json!(4096)
                } else {
                    json!(1024)
                },
                "Both real adapters must use the captured reasoning output allowance"
            );
            assert_eq!(
                input.get("tool_choice").cloned(),
                if case == "tool-call" {
                    Some(json!("required"))
                } else {
                    None
                },
                "Require the initial tool call without forcing a repeated call after its result"
            );
            let m = message(case);
            let delta = match case {
                "reasoning" => json!({"reasoning_content":"37 * 20 - 37","content":"703"}),
                "tool-call" => {
                    json!({"tool_calls":[{"index":0,"id":"p13-call","type":"function","function":{"name":"p13_echo","arguments":"{\"value\":\"P13_TOOL_7\"}"}}]})
                }
                _ => json!({"content":m["content"][0]["text"]}),
            };
            let first = format!(
                "data: {}\n\n",
                json!({"choices":[{"index":0,"delta":delta,"finish_reason":null}]})
            );
            if case == "cancellation" {
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{first}").unwrap();
                for _ in 0..200 {
                    std::thread::sleep(Duration::from_millis(2));
                    if socket.write_all(first.as_bytes()).is_err() {
                        break;
                    }
                }
            } else {
                let body = format!(
                    "{first}data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":if case=="tool-call"{"tool_calls"}else{"stop"}}],"usage":{"prompt_tokens":10,"completion_tokens":20}})
                );
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        }
    });
    let registry = Registry::at(&temp.path().join("registry")).unwrap();
    let bundle = Arc::new(
        RuntimeBundle::new(
            fixture::FakeHost::new(audit.clone()),
            fixture::plans(temp.path(), port, &["one", "two", "three"], 32768),
            registry.runtime_lease().unwrap(),
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let reference = Reference {
        node: material(&root.join(".local/toolchains/node-v24.14.0-linux-x64/bin/node")),
        probe: material(&root.join("tools/probes/p13_live_reference.mjs")),
        source: root.join("reference/ag1-executable"),
        materials: vec![material(&root.join(
            "reference/ag1-executable/packages/ai/dist/providers/openai-completions.js",
        ))],
    };
    let helper = material(Path::new(env!("CARGO_BIN_EXE_ag1-process-host")));
    let mut driver = TransportDriver::new(
        bundle.clone(),
        reference,
        helper,
        &temp.path().join("canary"),
    )
    .unwrap();
    let mut models = models();
    for model in &mut models {
        model["baseUrl"] = json!(format!("http://127.0.0.1:{port}/v1"));
    }
    let report = canary::rehearse(&mut driver, &models).await;
    done.store(true, Ordering::SeqCst);
    thread.join().unwrap();
    if report.status != "passed" {
        for file in fs::read_dir(temp.path().join("canary")).unwrap().flatten() {
            if file.path().extension().is_some_and(|x| x == "log") {
                eprintln!("{}", fs::read_to_string(file.path()).unwrap());
            }
        }
    }
    assert_eq!(
        report.status,
        "passed",
        "{:?}; last case: {:?}",
        report.failures,
        report.rows.last().map(|r| &r.case)
    );
    assert!(report.fixture);
    assert_eq!(requests.lock().unwrap().len(), 36);
    canary::verify_recording(&report, &models).unwrap();
    let events = bundle.events().unwrap();
    let expected: Vec<_> = models
        .iter()
        .flat_map(|model| {
            let id = model["id"].as_str().unwrap();
            (0..2).flat_map(move |_| {
                std::iter::once("owned_start")
                    .chain(std::iter::repeat_n("owned_ready", 6))
                    .chain(std::iter::once("owned_stop"))
                    .map(move |action| (id, action))
            })
        })
        .collect();
    assert_eq!(
        events
            .iter()
            .map(|e| (e.model.as_str(), e.action.as_str()))
            .collect::<Vec<_>>(),
        expected
    );
    assert!(audit.lock().unwrap().active.is_none());
    assert_eq!(audit.lock().unwrap().launches.len(), 6);
    assert_eq!(audit.lock().unwrap().maximum_active, 1);
    assert!(registry.runtime_lease().is_err());
    drop(driver);
    drop(bundle);
    assert!(registry.runtime_lease().is_ok());
}
