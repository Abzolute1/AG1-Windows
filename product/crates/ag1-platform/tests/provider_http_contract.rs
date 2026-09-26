use ag1_core::{DomainError, SecretBytes};
use ag1_core::{
    OperationId,
    inference::{EndpointKind, Scheduler},
    provider::{ApiFamily, ProviderEvent, StopReason},
};
use ag1_platform::provider::generation::*;
use ag1_platform::provider::http::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::Duration,
};
use tokio::time::Instant;
fn read_request(socket: &mut TcpStream) -> Vec<u8> {
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = vec![];
    let mut buffer = [0; 1024];
    loop {
        let count = socket.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..index]);
            let length = headers
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|n| n.parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= index + 4 + length {
                return bytes;
            }
        }
        assert!(bytes.len() < 65536);
    }
}
fn fixture<F: FnOnce(TcpStream) -> Vec<u8> + Send + 'static>(
    run: F,
) -> (String, thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        run(socket)
    });
    (origin, handle)
}
fn request(origin: &str, max: usize) -> HttpRequest {
    let mut headers = HostHeaders::default();
    headers
        .insert_secret(
            "authorization",
            &SecretBytes::new(b"Bearer fixture-token".to_vec()),
        )
        .unwrap();
    headers
        .insert_public("content-type", "application/json")
        .unwrap();
    HttpRequest {
        url: format!("{origin}/v1/chat/completions"),
        method: HttpMethod::Post,
        headers,
        body: b"{\"model\":\"fixture\"}".to_vec(),
        max_response_bytes: max,
    }
}
#[tokio::test]
async fn real_loopback_http_preserves_headers_body_and_fragmented_eof() {
    let (origin, server) = fixture(|mut socket| {
        let input = read_request(&mut socket);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
        for bytes in [b"data: ".as_slice(), "🌙\n\n".as_bytes()] {
            write!(socket, "{:x}\r\n", bytes.len()).unwrap();
            socket.write_all(bytes).unwrap();
            socket.write_all(b"\r\n").unwrap();
        }
        socket.write_all(b"0\r\n\r\n").unwrap();
        input
    });
    let observed = std::sync::Arc::new(std::sync::Mutex::new((Vec::new(), 0)));
    let captured = observed.clone();
    let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_request = sent.clone();
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1))
        .unwrap()
        .with_request_observer(std::sync::Arc::new(move |url, bytes| {
            captured_request
                .lock()
                .unwrap()
                .push((url.to_owned(), bytes.to_vec()));
            Ok(())
        }))
        .with_response_observer(std::sync::Arc::new(move |chunk| {
            let mut state = captured.lock().unwrap();
            if let Some(bytes) = chunk {
                state.0.extend_from_slice(bytes);
            } else {
                state.1 += 1;
            }
            Ok(())
        }));
    let (_source, mut cancel) = cancellation();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut response = http
        .send(request(&origin, 4096), deadline, &mut cancel)
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.content_type(), Some("text/event-stream"));
    assert!(!response.settled());
    let mut body = vec![];
    while let Some(chunk) = response.chunk(deadline, &mut cancel).await.unwrap() {
        body.extend(chunk);
    }
    assert_eq!(body, "data: 🌙\n\n".as_bytes());
    assert!(response.settled());
    assert!(
        response
            .chunk(deadline, &mut cancel)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(*observed.lock().unwrap(), (body.clone(), 1));
    let recorded = String::from_utf8(server.join().unwrap()).unwrap();
    assert!(recorded.contains("authorization: Bearer fixture-token\r\n"));
    assert!(recorded.ends_with("{\"model\":\"fixture\"}"));
    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, format!("{origin}/v1/chat/completions"));
    assert_eq!(sent[0].1, b"{\"model\":\"fixture\"}");
    assert!(recorded.as_bytes().ends_with(&sent[0].1));
}
#[tokio::test]
async fn redirects_never_forward_credentials_and_unconfigured_origins_fail_before_network() {
    let foreign = TcpListener::bind("127.0.0.1:0").unwrap();
    foreign.set_nonblocking(true).unwrap();
    let location = format!("http://{}/stolen", foreign.local_addr().unwrap());
    let (origin, server) = fixture(move |mut socket| {
        let input = read_request(&mut socket);
        write!(socket,"HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        input
    });
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
    let (_source, mut cancel) = cancellation();
    let deadline = Instant::now() + Duration::from_secs(3);
    assert!(matches!(
        http.send(
            request("https://unconfigured.invalid", 4096),
            deadline,
            &mut cancel
        )
        .await,
        Err(DomainError::PermissionDenied)
    ));
    let mut response = http
        .send(request(&origin, 4096), deadline, &mut cancel)
        .await
        .unwrap();
    assert_eq!(response.status(), 302);
    assert!(
        response
            .chunk(deadline, &mut cancel)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        foreign.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    server.join().unwrap();
}
#[tokio::test]
async fn ignored_server_cancellation_does_not_claim_transport_settlement() {
    let (done_tx, done_rx) = mpsc::channel();
    let (origin, server) = fixture(move |mut socket| {
        let input = read_request(&mut socket);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nx")
            .unwrap();
        let _ = done_rx.recv_timeout(Duration::from_secs(3));
        input
    });
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
    let (source, mut cancel) = cancellation();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut response = http
        .send(request(&origin, 4096), deadline, &mut cancel)
        .await
        .unwrap();
    assert_eq!(
        response.chunk(deadline, &mut cancel).await.unwrap(),
        Some(b"x".to_vec())
    );
    source.cancel();
    assert_eq!(
        response.chunk(deadline, &mut cancel).await,
        Err(DomainError::Cancelled)
    );
    assert!(!response.settled());
    done_tx.send(()).unwrap();
    server.join().unwrap();
}
#[tokio::test]
async fn bounded_body_and_slow_headers_return_explicit_errors() {
    let (origin, server) = fixture(|mut socket| {
        let input = read_request(&mut socket);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8192\r\nConnection: close\r\n\r\n")
            .unwrap();
        input
    });
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
    let (_source, mut cancel) = cancellation();
    assert!(matches!(
        http.send(
            request(&origin, 1024),
            Instant::now() + Duration::from_secs(3),
            &mut cancel
        )
        .await,
        Err(DomainError::LimitExceeded(_))
    ));
    server.join().unwrap();
    let (origin, server) = fixture(|mut socket| {
        let input = read_request(&mut socket);
        thread::sleep(Duration::from_millis(100));
        input
    });
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
    assert!(matches!(
        http.send(
            request(&origin, 1024),
            Instant::now() + Duration::from_millis(25),
            &mut cancel
        )
        .await,
        Err(DomainError::TimedOut)
    ));
    server.join().unwrap();
}
#[test]
fn origin_identity_and_header_validation_prevent_alias_and_injection_errors() {
    assert_eq!(
        endpoint_origin("HTTP://EXAMPLE.COM:80/v1/").unwrap(),
        "http://example.com"
    );
    assert_eq!(
        endpoint_origin("https://example.com:443/other").unwrap(),
        "https://example.com"
    );
    for value in [
        "file:///etc/passwd",
        "https://user:password@example.com",
        "http://localhost/#fragment",
    ] {
        assert!(endpoint_origin(value).is_err());
    }
    let mut headers = HostHeaders::default();
    assert!(
        headers
            .insert_public("x-fixture", "x\r\nAuthorization: leaked")
            .is_err()
    );
    assert!(headers.insert_public("host", "foreign").is_err());
}

#[tokio::test]
async fn subscription_stream_without_content_type_still_requires_valid_events_and_eof() {
    for valid in [true, false] {
        let body = if valid {
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"m\",\"role\":\"assistant\",\"content\":[]}}\n\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"m\",\"output_index\":0,\"content_index\":0,\"delta\":\"OK\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n"
        } else { "<html>not an event stream</html>" };
        let (origin, server) = fixture(move |mut socket| {
            let input=read_request(&mut socket);
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();input
        });
        let http=ProviderHttp::new(std::slice::from_ref(&origin),Duration::from_secs(1)).unwrap();
        let scheduler=Scheduler::default();scheduler.register(&origin,EndpointKind::Remote).unwrap();
        let ticket=scheduler.submit(&origin,OperationId::new("subscription").unwrap()).unwrap();let permit=scheduler.start(&ticket).unwrap().unwrap();
        let (_source,mut cancel)=cancellation();
        let result=request_generation(&http,GenerationRequest{request_id:"subscription".into(),tool_names:vec![],family:ApiFamily::CodexResponses,request:request(&origin,8192),limits:GenerationLimits{max_output_bytes:4096,prefill_ms:3000,idle_ms:3000,overall_ms:3000}},permit,&mut cancel,|_|Ok(())).await;
        if valid { let response=result.unwrap();assert_eq!(response.stop_reason,Some(StopReason::Stop));assert_eq!(response.content[0].text,"OK"); } else { assert!(result.is_err()); }
        server.join().unwrap();
    }
}

#[tokio::test]
async fn generation_pipeline_holds_one_permit_through_stop_usage_and_transport_eof() {
    for (reason, watchdog) in [("stop", 0), ("stop", 3000), ("length", 0), ("length", 3000)] {
        let body = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"hello\"}},\"finish_reason\":\"{reason}\"}}]}}\n\ndata: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":3}}}}\n\ndata: [DONE]\n\n"
        );
        let (origin, server) = fixture(move |mut socket| {
            let input = read_request(&mut socket);
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            input
        });
        let http =
            ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
        let (_source, mut cancel) = cancellation();
        let scheduler = Scheduler::default();
        scheduler.register(&origin, EndpointKind::Local).unwrap();
        let first = scheduler
            .submit(&origin, OperationId::new("first").unwrap())
            .unwrap();
        let permit = scheduler.start(&first).unwrap().unwrap();
        let next = scheduler
            .submit(&origin, OperationId::new("next").unwrap())
            .unwrap();
        let result = request_generation(
            &http,
            GenerationRequest {
                tool_names: vec!["read".into()],
                request_id: "fixture-operation".into(),
                family: ApiFamily::Completions,
                request: request(&origin, 8192),
                limits: GenerationLimits {
                    max_output_bytes: 4096,
                    prefill_ms: watchdog,
                    idle_ms: watchdog,
                    overall_ms: watchdog,
                },
            },
            permit,
            &mut cancel,
            |_| {
                assert!(scheduler.start(&next).unwrap().is_none());
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(
            result.stop_reason,
            Some(if reason == "length" {
                StopReason::Length
            } else {
                StopReason::Stop
            })
        );
        assert_eq!(result.usage.output, 3);
        assert_eq!(result.content[0].text, "hello");
        assert_eq!(scheduler.status(&origin).unwrap().quarantined, 0);
        scheduler.start(&next).unwrap().unwrap().terminal().unwrap();
        server.join().unwrap();
    }
}
#[tokio::test]
async fn generation_pipeline_quarantines_ignored_abort_and_unexpected_eof_with_partial_output() {
    for (aborted, watchdog) in [(false, 0), (false, 3000), (true, 0), (true, 3000)] {
        let (done_tx, done_rx) = mpsc::channel();
        let (origin, server) = fixture(move |mut socket| {
            let input = read_request(&mut socket);
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
            let length = body.len() + if aborted { 10 } else { 0 };
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}").unwrap();
            if aborted {
                let _ = done_rx.recv_timeout(Duration::from_secs(3));
            }
            input
        });
        let http =
            ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
        let (source, mut cancel) = cancellation();
        let scheduler = Scheduler::default();
        scheduler.register(&origin, EndpointKind::Local).unwrap();
        let first = scheduler
            .submit(&origin, OperationId::new("first").unwrap())
            .unwrap();
        let permit = scheduler.start(&first).unwrap().unwrap();
        let next = scheduler
            .submit(&origin, OperationId::new("next").unwrap())
            .unwrap();
        let failure = request_generation(
            &http,
            GenerationRequest {
                tool_names: vec!["read".into()],
                request_id: "fixture-operation".into(),
                family: ApiFamily::Completions,
                request: request(&origin, 8192),
                limits: GenerationLimits {
                    max_output_bytes: 4096,
                    prefill_ms: watchdog,
                    idle_ms: watchdog,
                    overall_ms: watchdog,
                },
            },
            permit,
            &mut cancel,
            |event| {
                if aborted && matches!(event, ProviderEvent::Delta { .. }) {
                    source.cancel();
                }
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert_eq!(failure.partial.unwrap().content[0].text, "partial");
        if aborted {
            assert_eq!(failure.error, DomainError::Cancelled);
        }
        assert_eq!(scheduler.status(&origin).unwrap().quarantined, 1);
        assert!(scheduler.start(&next).unwrap().is_none());
        let _ = done_tx.send(());
        server.join().unwrap();
        // The fixture task has now actually exited, supplying reconciliation evidence.
        scheduler.reconcile(&first).unwrap();
        scheduler.start(&next).unwrap().unwrap().terminal().unwrap();
    }
}

#[tokio::test]
async fn invalid_generation_releases_unused_permit_before_any_http() {
    let origin = "https://fixture.invalid".to_owned();
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
    let scheduler = Scheduler::default();
    scheduler.register(&origin, EndpointKind::Local).unwrap();
    let first = scheduler
        .submit(&origin, OperationId::new("invalid").unwrap())
        .unwrap();
    let permit = scheduler.start(&first).unwrap().unwrap();
    let next = scheduler
        .submit(&origin, OperationId::new("next").unwrap())
        .unwrap();
    let (_source, mut cancel) = cancellation();
    let result = request_generation(
        &http,
        GenerationRequest {
            tool_names: vec!["read".into()],
            request_id: "invalid".into(),
            family: ApiFamily::Completions,
            request: request(&origin, 1024),
            limits: GenerationLimits {
                max_output_bytes: 0,
                prefill_ms: 100,
                idle_ms: 100,
                overall_ms: 1000,
            },
        },
        permit,
        &mut cancel,
        |_| Ok(()),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(scheduler.status(&origin).unwrap().quarantined, 0);
    scheduler.start(&next).unwrap().unwrap().terminal().unwrap();
}
#[tokio::test]
async fn nvidia_preflight_and_vision_canary_each_hold_one_shared_permit() {
    use ag1_platform::provider::capability::*;
    for canary in [false, true] {
        let (origin, server) = fixture(move |mut socket| {
            let input = read_request(&mut socket);
            let body = if canary {
                r#"{"choices":[{"message":{"content":"teal"}}]}"#
            } else {
                "forbidden"
            };
            let status = if canary { "200 OK" } else { "403 Forbidden" };
            write!(socket,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            input
        });
        let http =
            ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
        let scheduler = Scheduler::default();
        scheduler.register(&origin, EndpointKind::Local).unwrap();
        let first = scheduler
            .submit(&origin, OperationId::new("probe").unwrap())
            .unwrap();
        let permit = scheduler.start(&first).unwrap().unwrap();
        let next = scheduler
            .submit(&origin, OperationId::new("root").unwrap())
            .unwrap();
        assert!(scheduler.start(&next).unwrap().is_none());
        let (_source, mut cancel) = cancellation();
        if canary {
            let result = probe_canary(
                &http,
                &format!("{origin}/v1"),
                "fixture",
                None,
                permit,
                &mut cancel,
            )
            .await
            .unwrap();
            assert_eq!(result.verdict, ag1_core::model::VisionVerdict::Yes);
        } else {
            assert_eq!(
                nvidia_preflight(
                    &http,
                    request(&origin, 4096),
                    "fixture",
                    permit,
                    &mut cancel
                )
                .await
                .unwrap(),
                403
            );
        }
        scheduler.start(&next).unwrap().unwrap().terminal().unwrap();
        let input = server.join().unwrap();
        let offset = input.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
        let body: serde_json::Value = serde_json::from_slice(&input[offset..]).unwrap();
        assert_eq!(body["model"], "fixture");
        assert_eq!(
            body[if canary {
                "max_tokens"
            } else {
                "max_completion_tokens"
            }],
            if canary { 512 } else { 1 }
        );
    }
}

#[tokio::test]
async fn runtime_http_observation_checks_loaded_props_and_never_adopts_listener() {
    use ag1_platform::llama::{lifecycle::Observation, observation::observe};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for i in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let input = read_request(&mut socket);
            let path = if i == 0 { "/health" } else { "/props" };
            assert!(String::from_utf8_lossy(&input).starts_with(&format!("GET {path} ")));
            let body = if i == 0 {
                "{}"
            } else {
                r#"{"model_path":"/fixture/model.gguf","model_alias":"fixture","default_generation_settings":{"model":"fixture","n_ctx":90000},"modalities":{"vision":false}}"#
            };
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
    let observed = observe(&http, &origin).await.unwrap();
    let Observation::Healthy(id) = observed else {
        panic!("missing runtime identity")
    };
    assert_eq!(id.path.as_deref(), Some("/fixture/model.gguf"));
    assert_eq!(id.context_window, Some(90000));
    assert!(!id.vision);
    server.join().unwrap();
    assert_eq!(observe(&http, &origin).await.unwrap(), Observation::Vacant);
    let (origin, server) = fixture(|mut socket| {
        let input = read_request(&mut socket);
        socket
            .write_all(
                b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        input
    });
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
    assert_eq!(
        observe(&http, &origin).await.unwrap(),
        Observation::Occupied
    );
    server.join().unwrap();
}

#[tokio::test]
async fn upstream_errors_cannot_echo_request_credentials_into_provider_events() {
    for sse in [false, true] {
        let (origin, server) = fixture(move |mut socket| {
            let input = read_request(&mut socket);
            let body = if sse {
                "data: {\"error\":{\"message\":\"invalid Bearer fixture-token / fixture-token\"}}\n\n"
            } else {
                "invalid Bearer fixture-token / fixture-token"
            };
            let (status, kind) = if sse {
                ("200 OK", "text/event-stream")
            } else {
                ("401 Unauthorized", "application/json")
            };
            write!(socket,"HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            input
        });
        let http =
            ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
        let scheduler = Scheduler::default();
        scheduler.register(&origin, EndpointKind::Local).unwrap();
        let ticket = scheduler
            .submit(&origin, OperationId::new("credential-error").unwrap())
            .unwrap();
        let permit = scheduler.start(&ticket).unwrap().unwrap();
        let (_source, mut cancel) = cancellation();
        let result = request_generation(
            &http,
            GenerationRequest {
                tool_names: vec![],
                request_id: "credential-error".into(),
                family: ApiFamily::Completions,
                request: request(&origin, 4096),
                limits: GenerationLimits {
                    max_output_bytes: 4096,
                    prefill_ms: 1000,
                    idle_ms: 1000,
                    overall_ms: 2000,
                },
            },
            permit,
            &mut cancel,
            |event| {
                assert!(!format!("{event:?}").contains("fixture-token"));
                Ok(())
            },
        )
        .await
        .unwrap();
        assert!(!result.error.unwrap().contains("fixture-token"));
        server.join().unwrap();
    }
}

#[tokio::test]
async fn cancelled_generation_releases_permit_only_after_positive_owned_stop_proof() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct OwnedProof {
        stopped: Arc<AtomicBool>,
        origin: String,
    }
    impl GenerationRuntime for OwnedProof {
        fn prepare<'a>(&'a self, _: &'a HttpRequest) -> ag1_core::PortFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
        fn shutdown(&self) -> ag1_core::PortFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn cancel_generation<'a>(&'a self, url: &'a str) -> ag1_core::PortFuture<'a, ()> {
            Box::pin(async move {
                assert_eq!(endpoint_origin(url).unwrap(), self.origin);
                self.stopped.store(true, Ordering::Release);
                Ok(())
            })
        }
    }
    let stopped = Arc::new(AtomicBool::new(false));
    let server_stopped = stopped.clone();
    let (origin, server) = fixture(move |mut socket| {
        let input = read_request(&mut socket);
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()+10).unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while !server_stopped.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < until);
            thread::sleep(Duration::from_millis(5));
        }
        input
    });
    let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1))
        .unwrap()
        .with_runtime(Arc::new(OwnedProof {
            stopped: stopped.clone(),
            origin: origin.clone(),
        }));
    let scheduler = Scheduler::default();
    scheduler.register(&origin, EndpointKind::Local).unwrap();
    let first = scheduler
        .submit(&origin, OperationId::new("cancel-owned").unwrap())
        .unwrap();
    let permit = scheduler.start(&first).unwrap().unwrap();
    let (source, mut cancel) = cancellation();
    let result = request_generation(
        &http,
        GenerationRequest {
            tool_names: vec![],
            request_id: "owned".into(),
            family: ApiFamily::Completions,
            request: request(&origin, 8192),
            limits: GenerationLimits {
                max_output_bytes: 4096,
                prefill_ms: 1000,
                idle_ms: 1000,
                overall_ms: 3000,
            },
        },
        permit,
        &mut cancel,
        |event| {
            if matches!(event, ProviderEvent::Delta { .. }) {
                source.cancel();
            }
            Ok(())
        },
    )
    .await
    .unwrap_err();
    assert_eq!(result.error, DomainError::Cancelled);
    assert_eq!(result.partial.unwrap().content[0].text, "partial");
    assert!(stopped.load(Ordering::Acquire));
    assert_eq!(scheduler.status(&origin).unwrap().quarantined, 0);
    let next = scheduler
        .submit(&origin, OperationId::new("after-cancel").unwrap())
        .unwrap();
    scheduler.start(&next).unwrap().unwrap().terminal().unwrap();
    server.join().unwrap();
}

#[tokio::test]
async fn remote_generation_abort_and_unexpected_eof_release_client_capacity_without_success() {
    for (aborted, watchdog) in [(false, 0), (false, 3000), (true, 0), (true, 3000)] {
        let (done_tx, done_rx) = mpsc::channel();
        let (origin, server) = fixture(move |mut socket| {
            let input = read_request(&mut socket);
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
            let length = body.len() + if aborted { 10 } else { 0 };
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}").unwrap();
            if aborted {
                let _ = done_rx.recv_timeout(Duration::from_secs(3));
            }
            input
        });
        let http =
            ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(1)).unwrap();
        let (source, mut cancel) = cancellation();
        let scheduler = Scheduler::default();
        scheduler.register(&origin, EndpointKind::Remote).unwrap();
        let first = scheduler
            .submit(&origin, OperationId::new("first").unwrap())
            .unwrap();
        let permit = scheduler.start(&first).unwrap().unwrap();
        let next = scheduler
            .submit(&origin, OperationId::new("next").unwrap())
            .unwrap();
        let failure = request_generation(
            &http,
            GenerationRequest {
                tool_names: vec!["read".into()],
                request_id: "fixture-operation".into(),
                family: ApiFamily::Completions,
                request: request(&origin, 8192),
                limits: GenerationLimits {
                    max_output_bytes: 4096,
                    prefill_ms: watchdog,
                    idle_ms: watchdog,
                    overall_ms: watchdog,
                },
            },
            permit,
            &mut cancel,
            |event| {
                if aborted && matches!(event, ProviderEvent::Delta { .. }) {
                    source.cancel();
                }
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert_eq!(failure.partial.unwrap().content[0].text, "partial");
        if aborted {
            assert_eq!(failure.error, DomainError::Cancelled);
        }
        assert_eq!(scheduler.status(&origin).unwrap().quarantined, 0);
        assert!(scheduler.ticket_settled(&first).unwrap());
        let _ = done_tx.send(());
        server.join().unwrap();
        // The fixture task has now actually exited, supplying reconciliation evidence.
        assert_eq!(scheduler.reconcile(&first), Err(DomainError::NotFound));
        scheduler.start(&next).unwrap().unwrap().terminal().unwrap();
    }
}
