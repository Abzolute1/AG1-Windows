use super::*;

#[derive(Default)]
struct ReceiptGate {
    entered: Notify,
    released: Mutex<bool>,
    changed: std::sync::Condvar,
}
impl ReceiptGate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
impl ag1_storage::FaultInjector for ReceiptGate {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        point: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if command == "choose-model" && point == ag1_storage::Boundary::BeforeAck {
            self.entered.notify_one();
            let (released, _) = self
                .changed
                .wait_timeout_while(
                    self.released.lock().unwrap(),
                    Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            assert!(*released, "release model acknowledgement");
        }
        Ok(())
    }
}

async fn returning_output_waits(drop_receipt: bool) {
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", socket.local_addr().unwrap());
    let requested = Arc::new(Notify::new());
    let entered = requested.clone();
    let (release, wait) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..if drop_receipt { 1 } else { 2 } {
            let until = Instant::now() + Duration::from_secs(5);
            let mut client = loop {
                match socket.accept() {
                    Ok((client, _)) => break client,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < until, "expected native request");
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            requests.push(read_request(&mut client));
            if index == 0 {
                entered.notify_one();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            let reply = if index == 0 {
                calls(vec![(
                    "write-after-receipt",
                    "write",
                    json!({"path":"receipt.txt","content":"once"}),
                )])
            } else {
                stop()
            };
            let body = format!("data: {reply}\n\ndata: [DONE]\n\n");
            write!(client,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
        requests
    });
    let gate = Arc::new(ReceiptGate::default());
    let fixture = fixture_configured(&origin, "disabled", vec![], Some(gate.clone()), None, 1);
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let mut run = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), files, &ignore)
            .await
    });
    requested.notified().await;
    let old_revision = fixture.host.journal_revision().unwrap();
    let mut model = fixture.model.clone();
    model["id"] = json!("replacement");
    // Own the actual future so drop below abandons its receipt, not just a Pin
    // reference. One poll submits SQL; the fixture gate prevents inline finish.
    let mut control =
        Box::pin(
            fixture
                .host
                .select_model("choose-model", "choose-model", NOW, model, &ignore),
        );
    std::future::poll_fn(|cx| {
        assert!(control.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    gate.entered.notified().await;
    gate.release();
    // Reading the result also waits for the SQL writer to leave the gate. The
    // caller future is deliberately not polled to publish the in-memory receipt.
    assert!(
        fixture
            .store
            .command_result("choose-model")
            .unwrap()
            .is_some()
    );
    assert_eq!(fixture.host.journal_revision().unwrap(), old_revision);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if provider_outputs(&fixture.store)
                .iter()
                .any(|v| v["kind"] == "runtime-provider-output")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut run)
            .await
            .is_err()
    );
    assert!(!fixture.root.join("receipt.txt").exists());
    if drop_receipt {
        drop(control);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), run)
                .await
                .unwrap()
                .unwrap(),
            Err(DomainError::DurabilityUnknown)
        ));
        assert!(!fixture.host.persistence_healthy().unwrap());
        assert!(!fixture.root.join("receipt.txt").exists());
        assert_eq!(fixture.host.journal_revision().unwrap(), old_revision);
    } else {
        control.await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.generations, 2);
        assert!(fixture.host.persistence_healthy().unwrap());
        assert!(fixture.host.settled().unwrap());
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("receipt.txt")).unwrap(),
            "once"
        );
    }
    let requests = server.join().unwrap();
    assert_eq!(requests[0]["model"], "initial");
    if !drop_receipt {
        assert_eq!(requests[1]["model"], "replacement");
    }
    assert_eq!(fixture.scheduler.status(&origin).unwrap().active, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn returning_provider_waits_for_control_receipt_before_tools_and_next_request() {
    returning_output_waits(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoned_control_receipt_wakes_returning_provider_into_recovery() {
    returning_output_waits(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_worker_keeps_control_acknowledgement_when_caller_drops_response() {
    let (origin, server) = server(vec![
        calls(vec![(
            "pending-write",
            "write",
            json!({"path":"receipt.txt","content":"no"}),
        )]),
        stop(),
    ]);
    let gate = Arc::new(ReceiptGate::default());
    let mut fixture =
        fixture_configured(&origin, "interactive", vec![], Some(gate.clone()), None, 1);
    let approval = Arc::new(WaitingApproval {
        entered: Notify::new(),
        approved: AtomicBool::new(false),
    });
    Arc::get_mut(&mut fixture.files).unwrap().approvals = approval.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), files, &ignore)
            .await
    });
    approval.entered.notified().await;
    let mut model = fixture.model.clone();
    model["id"] = json!("replacement");
    let mut control =
        Box::pin(
            fixture
                .host
                .select_model("choose-model", "choose-model", NOW, model, &ignore),
        );
    std::future::poll_fn(|cx| {
        assert!(control.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    gate.entered.notified().await;
    drop(control);
    // Here the accepted tool worker applies the receipt itself. Losing the
    // caller's response is not loss of the internal acknowledgement owner.
    assert!(fixture.host.persistence_healthy().unwrap());
    gate.release();
    tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(fixture.host.persistence_healthy().unwrap());
    assert!(fixture.host.settled().unwrap());
    assert!(!fixture.root.join("receipt.txt").exists());
    let requests = server.join().unwrap();
    assert_eq!(requests[1]["model"], "replacement");
}
