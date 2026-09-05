use ag1_protocol::{fake::*, *};
use base64::Engine;
use serde_json::{Value, json};

fn host() -> FakeHost {
    FakeHost::new("test-instance".to_owned().try_into().unwrap())
}
fn prompt(host: &FakeHost, id: &str) -> CommandEnvelope {
    host.envelope(
        id,
        Command::Prompt {
            input: PromptInput::Inline {
                text: "fixture".into(),
            },
        },
    )
}
fn operation(reply: Reply) -> OperationId {
    match reply.result {
        CommandResult::Accepted { operation } => operation,
        other => panic!("unexpected {other:?}"),
    }
}
fn error(reply: Reply) -> ProtocolError {
    match reply.result {
        CommandResult::Rejected { error } => error,
        other => panic!("unexpected {other:?}"),
    }
}
fn events(delivery: Delivery) -> Vec<Event> {
    match delivery {
        Delivery::Events { events, .. } => events,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn decimal_sequences_and_optional_null_custom_payloads_roundtrip() {
    for n in [0, 9_007_199_254_740_993, u64::MAX] {
        let seq = Sequence::new(n);
        assert_eq!(
            serde_json::from_str::<Sequence>(&serde_json::to_string(&seq).unwrap())
                .unwrap()
                .get(),
            n
        );
    }
    for invalid in ["00", "-1", "+1", "1e3", "18446744073709551616", "", " 1"] {
        assert!(serde_json::from_value::<Sequence>(json!(invalid)).is_err());
    }
    assert!(serde_json::from_value::<Sequence>(json!(9_007_199_254_740_993u64)).is_err());
    let base = serde_json::to_value(prompt(&host(), "nulls")).unwrap();
    for extension in [
        None,
        Some(Value::Null),
        Some(json!({"future": [null, 1, {"k": true}]})),
    ] {
        let mut value = base.clone();
        if let Some(ext) = extension.clone() {
            value["extension"] = ext;
        }
        let decoded: CommandEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded.extension, extension);
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
}

#[test]
fn rejects_incompatible_major_schema_and_accepts_minor_negotiation() {
    let host = host();
    let mut version = Version::default();
    assert!(host.handshake(&version).unwrap().fake);
    version.minor = 500;
    assert_eq!(host.handshake(&version).unwrap().version.minor, 0);
    version.major = 2;
    assert_eq!(
        host.handshake(&version),
        Err(ProtocolError::IncompatibleVersion)
    );
    version.major = 1;
    version.schema = "different".into();
    assert_eq!(
        host.handshake(&version),
        Err(ProtocolError::IncompatibleVersion)
    );
}

#[test]
fn snapshot_subscription_barrier_and_duplicate_event_projection() {
    let mut host = host();
    host.command(prompt(&host, "before"));
    let sub = host.subscribe().unwrap();
    let mut projection = Projection::new(sub.snapshot);
    host.command(prompt(&host, "after"));
    host.step();
    let tail = events(host.poll(&sub.id).unwrap());
    assert_eq!(tail.len(), 2);
    for event in &tail {
        assert_eq!(projection.apply(event), Ok(true));
        assert_eq!(projection.apply(event), Ok(false));
    }
    assert_eq!(projection.snapshot, host.snapshot());
}

#[test]
fn out_of_order_delivery_requires_snapshot_before_more_mutations() {
    let mut host = host();
    let sub = host.subscribe().unwrap();
    let mut projection = Projection::new(sub.snapshot);
    host.command(prompt(&host, "one"));
    host.step();
    let tail = events(host.poll(&sub.id).unwrap());
    let before = projection.snapshot.clone();
    assert_eq!(
        projection.apply(&tail[1]),
        Err(ProtocolError::ResyncRequired)
    );
    assert_eq!(
        projection.apply(&tail[0]),
        Err(ProtocolError::ResyncRequired)
    );
    assert_eq!(projection.snapshot, before);
    let fresh = Projection::new(host.snapshot());
    assert!(!fresh.needs_resync);
}

#[test]
fn duplicate_command_returns_prior_admission_without_reexecution() {
    let mut host = host();
    let command = prompt(&host, "effect");
    let first = host.command(command.clone());
    assert!(host.step());
    assert_eq!(host.command(command.clone()).result, first.result);
    assert!(!host.step());
    assert_eq!(host.snapshot().watermark.get(), 2);
    let mut conflict = command;
    conflict.command = Command::Prompt {
        input: PromptInput::Inline {
            text: "different effect".into(),
        },
    };
    assert_eq!(
        error(host.command(conflict)),
        ProtocolError::CommandConflict
    );
    assert!(matches!(
        host.lookup(&"effect".to_owned().try_into().unwrap()),
        CommandResult::Settled {
            operation: OperationView {
                status: OperationStatus::Completed,
                ..
            }
        }
    ));
}

#[test]
fn restart_rejects_old_commands_cursors_and_late_old_events() {
    let mut host = host();
    let old = prompt(&host, "old");
    let sub = host.subscribe().unwrap();
    host.command(old.clone());
    let event = events(host.poll(&sub.id).unwrap()).remove(0);
    assert!(
        host.restart("test-instance".to_owned().try_into().unwrap())
            .is_err()
    );
    host.restart("next-instance".to_owned().try_into().unwrap())
        .unwrap();
    let mut projection = Projection::new(host.snapshot());
    let before = projection.snapshot.clone();
    assert_eq!(projection.apply(&event), Err(ProtocolError::StaleContext));
    assert_eq!(projection.snapshot, before);
    assert_eq!(error(host.command(old)), ProtocolError::StaleContext);
    assert!(matches!(
        host.replay(&Cursor {
            scope: event.scope,
            after: event.sequence
        }),
        Delivery::Gap { .. }
    ));
    assert!(!host.step());
    assert_eq!(host.snapshot().scope.epoch.get(), 2);
}

#[test]
fn approval_binds_arguments_revision_epoch_expiry_and_is_single_use() {
    for change in [
        "argument",
        "operation",
        "revision",
        "epoch",
        "workspace",
        "expiry",
        "id",
    ] {
        let mut host = host();
        let op = operation(host.command(prompt(&host, "work")));
        let binding = host.require_approval(op, "a".repeat(64), 100).unwrap();
        assert!(!host.step());
        let mut wrong = binding.clone();
        match change {
            "argument" => wrong.argument_sha256 = "b".repeat(64),
            "operation" => wrong.operation = "other".to_owned().try_into().unwrap(),
            "revision" => wrong.revision = Sequence::new(0),
            "epoch" => wrong.scope.epoch = Sequence::new(0),
            "workspace" => wrong.scope.workspace = "other".to_owned().try_into().unwrap(),
            "expiry" => wrong.expires_ms = Sequence::new(101),
            _ => wrong.id = "invented".to_owned().try_into().unwrap(),
        }
        assert_eq!(
            error(host.command(host.envelope(
                "wrong",
                Command::Approve {
                    binding: wrong,
                    allow: true
                }
            ))),
            ProtocolError::InvalidApproval
        );
        let allow = host.envelope(
            "allow",
            Command::Approve {
                binding: binding.clone(),
                allow: true,
            },
        );
        assert!(matches!(
            host.command(allow.clone()).result,
            CommandResult::Settled { .. }
        ));
        assert!(matches!(
            host.command(allow).result,
            CommandResult::Settled { .. }
        )); // same request retry is prior admission
        assert_eq!(
            error(host.command(host.envelope(
                "duplicate-answer",
                Command::Approve {
                    binding,
                    allow: true
                }
            ))),
            ProtocolError::InvalidApproval
        );
        assert!(host.step());
        assert!(!host.step());
    }
    let mut host = host();
    let op = operation(host.command(prompt(&host, "work")));
    let binding = host.require_approval(op, "a".repeat(64), 100).unwrap();
    host.advance_ms(100);
    assert_eq!(
        error(host.command(host.envelope(
            "expired",
            Command::Approve {
                binding,
                allow: true
            }
        ))),
        ProtocolError::InvalidApproval
    );
    assert!(!host.step());
}

#[test]
fn approval_stales_after_revision_change_and_cancel_settles_blocked_work() {
    let mut host = host();
    let op = operation(host.command(prompt(&host, "work")));
    let binding = host
        .require_approval(op.clone(), "a".repeat(64), 100)
        .unwrap();
    host.command(prompt(&host, "revision-change"));
    assert_eq!(
        error(host.command(host.envelope(
            "stale",
            Command::Approve {
                binding,
                allow: true
            }
        ))),
        ProtocolError::InvalidApproval
    );
    let result = host.command(host.envelope("cancel", Command::Cancel { operation: op }));
    assert!(matches!(
        result.result,
        CommandResult::Settled {
            operation: OperationView {
                status: OperationStatus::Cancelled,
                ..
            }
        }
    ));
    assert!(host.step());
}

#[test]
fn session_and_endpoint_queues_reject_at_capacity_and_preserve_fifo() {
    let mut host = host();
    let mut first = None;
    for i in 0..MAX_SESSION_QUEUE {
        let op = operation(host.command(prompt(&host, &format!("q{i}"))));
        if i == 0 {
            first = Some(op);
        }
    }
    assert!(matches!(
        error(host.command(prompt(&host, "overflow"))),
        ProtocolError::QueueFull { capacity: 256, .. }
    ));
    assert_eq!(host.snapshot().queued.len(), 256);
    assert!(host.step());
    assert_eq!(
        host.snapshot().last_terminal.unwrap().operation,
        first.unwrap()
    );
    operation(host.command(prompt(&host, "now-fits")));
    let mut queue = ag1_core::EndpointQueue::new(MAX_ENDPOINT_WAITING);
    for i in 0..128 {
        queue.admit(i).unwrap();
    }
    assert_eq!(queue.admit(128), Err(ag1_core::DomainError::QueueFull));
    queue.cancel(|i| *i == 64);
    queue.admit(128).unwrap();
    let actual: Vec<_> = std::iter::from_fn(|| queue.pop()).collect();
    assert_eq!(actual, (0..129).filter(|i| *i != 64).collect::<Vec<_>>());
}

#[test]
fn paused_subscriber_is_bounded_reports_gap_and_recovers_terminal_state() {
    let mut host = host();
    let sub = host.subscribe().unwrap();
    let op = operation(host.command(prompt(&host, "stream")));
    for _ in 0..3_000 {
        host.advance_ms(10);
        host.progress(&op, "x".repeat(2048)).unwrap();
        assert!(host.queued_bytes(&sub.id).unwrap() <= MAX_SUBSCRIBER_BYTES);
    }
    host.step();
    let Delivery::Gap { snapshot } = host.poll(&sub.id).unwrap() else {
        panic!("lost progress needs explicit gap");
    };
    assert_eq!(
        snapshot.last_terminal.unwrap().status,
        OperationStatus::Completed
    );
    assert_eq!(host.queued_bytes(&sub.id), Some(0));
    assert!(events(host.poll(&sub.id).unwrap()).is_empty());
    assert!(host.peak_subscriber_bytes <= MAX_SUBSCRIBER_BYTES);
    println!(
        "30s simulated pause: peak transient bytes={}",
        host.peak_subscriber_bytes
    );
}

#[test]
fn replay_is_paged_and_retention_gaps_require_resnapshot() {
    let mut host = host();
    let sub = host.subscribe().unwrap();
    for i in 0..600 {
        host.command(prompt(&host, &format!("r{i}")));
        host.step();
    }
    assert!(matches!(host.poll(&sub.id).unwrap(), Delivery::Gap { .. }));
    let mut cursor = Cursor {
        scope: host.snapshot().scope,
        after: Sequence::new(1000),
    };
    let mut count = 0;
    loop {
        let delivery = host.replay(&cursor);
        assert!(
            encode(&WireMessage::Delivery(delivery.clone()))
                .unwrap()
                .len()
                <= MAX_ENVELOPE
        );
        let Delivery::Events {
            events,
            cursor: next,
            more,
        } = delivery
        else {
            panic!();
        };
        assert!(events.len() <= 32);
        count += events.len();
        cursor = next;
        if !more {
            break;
        }
    }
    assert_eq!(count, 200);
    cursor.after = Sequence::new(u64::MAX);
    assert!(matches!(host.replay(&cursor), Delivery::Gap { .. }));
}

#[test]
fn large_prompt_uses_artifact_and_binary_pages_fit_json_limit() {
    let mut host = host();
    let artifact: ArtifactId = "upload-1".to_owned().try_into().unwrap();
    let oversized = host.envelope(
        "large",
        Command::Prompt {
            input: PromptInput::Inline {
                text: "x".repeat(MAX_ENVELOPE),
            },
        },
    );
    assert!(matches!(
        error(host.command(oversized)),
        ProtocolError::Oversize {
            use_artifact: true,
            ..
        }
    ));
    host.install_artifact(artifact.clone(), vec![255; 100_000])
        .unwrap();
    operation(host.command(host.envelope(
        "reference",
        Command::Prompt {
            input: PromptInput::Artifact {
                artifact: artifact.clone(),
            },
        },
    )));
    let read = host.envelope(
        "page",
        Command::ArtifactRead {
            artifact: artifact.clone(),
            offset: Sequence::new(0),
            limit: MAX_ARTIFACT_PAGE,
        },
    );
    let reply = host.command(read);
    assert!(encode(&WireMessage::Result(reply.clone())).unwrap().len() < MAX_ENVELOPE);
    let CommandResult::ArtifactPage { base64, next, .. } = reply.result else {
        panic!();
    };
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(base64)
            .unwrap(),
        vec![255; MAX_ARTIFACT_PAGE as usize]
    );
    assert_eq!(next.unwrap().get(), 32768);
    assert!(matches!(
        error(host.command(host.envelope(
            "too-large",
            Command::ArtifactRead {
                artifact: artifact.clone(),
                offset: Sequence::new(0),
                limit: MAX_ARTIFACT_PAGE + 1
            }
        ))),
        ProtocolError::Oversize { .. }
    ));
    assert_eq!(
        error(host.command(host.envelope(
            "invalid-offset",
            Command::ArtifactRead {
                artifact,
                offset: Sequence::new(u64::MAX),
                limit: 10
            }
        ))),
        ProtocolError::NotFound
    );
}

#[test]
fn malformed_oversize_unknown_fields_and_bounded_parser_fuzz() {
    for bytes in [
        vec![],
        b"null".to_vec(),
        b"{}".to_vec(),
        vec![255; 16],
        vec![b' '; MAX_ENVELOPE + 1],
        format!("{}0{}", "[".repeat(200), "]".repeat(200)).into_bytes(),
    ] {
        assert!(decode(&bytes).is_err());
    }
    let command = WireMessage::Command(prompt(&host(), "valid"));
    let bytes = encode(&command).unwrap();
    assert_eq!(decode(&bytes).unwrap(), command);
    for end in 0..bytes.len() {
        assert!(decode(&bytes[..end]).is_err());
    }
    let mut value = serde_json::to_value(&command).unwrap();
    value["data"]["surprise"] = json!(true);
    assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
    let mut seed = 0x12345678u64;
    for len in 0..2048 {
        let garbage: Vec<u8> = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                (seed >> 32) as u8
            })
            .collect();
        let _ = decode(&garbage); // deterministic fuzz: no panic, bounded length/depth
    }
}

#[test]
fn unavailable_operations_never_claim_success_and_scope_is_enforced() {
    let mut host = host();
    for command in [
        Command::Project {
            action: "open".into(),
            args: json!({}),
        },
        Command::Session {
            action: "fork".into(),
            args: json!({}),
        },
        Command::Task {
            action: "set".into(),
            args: json!({}),
        },
        Command::Plan {
            action: "set".into(),
            args: json!({}),
        },
        Command::Files {
            action: "save".into(),
            args: json!({}),
        },
        Command::Artifacts {
            action: "list".into(),
            args: json!({}),
        },
        Command::Memory {
            action: "write".into(),
            args: json!({}),
        },
        Command::Model {
            action: "load".into(),
            args: json!({}),
        },
        Command::Runtime {
            action: "launch".into(),
            args: json!({}),
        },
        Command::Diagnostics {
            action: "export".into(),
            args: json!({}),
        },
    ] {
        assert!(matches!(
            error(host.command(host.envelope("unsupported", command))),
            ProtocolError::Unavailable { .. }
        ));
    }
    assert_eq!(host.snapshot().watermark.get(), 0);
    let mut wrong = prompt(&host, "cross-session");
    wrong.scope.session = "other".to_owned().try_into().unwrap();
    assert_eq!(error(host.command(wrong)), ProtocolError::StaleContext);
}

#[test]
fn subscriber_and_artifact_registry_limits_are_explicit() {
    let mut host = host();
    let mut subscriptions = vec![];
    for _ in 0..MAX_SUBSCRIBERS {
        subscriptions.push(host.subscribe().unwrap());
    }
    assert!(matches!(
        host.subscribe(),
        Err(ProtocolError::Capacity { .. })
    ));
    host.unsubscribe(&subscriptions[0].id);
    assert!(host.subscribe().is_ok());
    host.install_artifact(
        "full".to_owned().try_into().unwrap(),
        vec![0; MAX_FIXTURE_ARTIFACT_BYTES],
    )
    .unwrap();
    assert!(
        host.install_artifact("extra".to_owned().try_into().unwrap(), vec![1])
            .is_err()
    );
}

#[test]
fn nullable_fields_are_required_and_dispatch_correlates_results() {
    let mut host = host();
    let mut snapshot = serde_json::to_value(host.snapshot()).unwrap();
    snapshot.as_object_mut().unwrap().remove("last_terminal");
    assert!(serde_json::from_value::<Snapshot>(snapshot).is_err());
    let command = prompt(&host, "correlation");
    let id = command.id.clone();
    let reply = decode(
        &host
            .dispatch(&encode(&WireMessage::Command(command)).unwrap())
            .unwrap(),
    )
    .unwrap();
    let WireMessage::Result(reply) = reply else {
        panic!();
    };
    assert_eq!(reply.id, id);
    assert_eq!(reply.watermark.get(), 1);
    assert_eq!(reply.scope, host.snapshot().scope);
    assert!(
        host.dispatch(&encode(&WireMessage::Snapshot(host.snapshot())).unwrap())
            .is_err()
    );
}

#[test]
fn retained_admissions_never_evict_idempotency_and_leave_cancellation_capacity() {
    let mut host = host();
    let first = prompt(&host, "first");
    let accepted = host.command(first.clone());
    host.step();
    for i in 1..MAX_COMMAND_RECORDS - 2 * MAX_SESSION_QUEUE - 1 {
        operation(host.command(prompt(&host, &format!("history-{i}"))));
        host.step();
    }
    let op = operation(host.command(prompt(&host, "last")));
    assert!(matches!(
        error(host.command(prompt(&host, "over-history-limit"))),
        ProtocolError::Capacity { .. }
    ));
    assert_eq!(host.command(first).result, accepted.result);
    assert!(matches!(
        host.command(host.envelope("cancel-at-capacity", Command::Cancel { operation: op }))
            .result,
        CommandResult::Settled {
            operation: OperationView {
                status: OperationStatus::Cancelled,
                ..
            }
        }
    ));
    assert!(!host.step());
}

#[test]
fn records_controlled_fixture_dispatch_measurements() {
    let mut host = host();
    let mut nanos = Vec::new();
    let mut peak_payload = 0;
    for i in 0..1000 {
        let request = encode(&WireMessage::Command(prompt(
            &host,
            &format!("measure-{i}"),
        )))
        .unwrap();
        let start = std::time::Instant::now();
        let response = host.dispatch(&request).unwrap();
        assert!(matches!(
            decode(&response).unwrap(),
            WireMessage::Result(Reply {
                result: CommandResult::Accepted { .. },
                ..
            })
        ));
        nanos.push(start.elapsed().as_nanos());
        peak_payload = peak_payload.max(request.len()).max(response.len());
        host.step();
    }
    nanos.sort_unstable();
    println!(
        "P02_MEASUREMENT {}",
        json!({"dispatches":1000,"events":2000,"max_envelope_bytes":peak_payload,"dispatch_p95_ns":nanos[949],"dispatch_max_ns":nanos[999],"mode":"debug, in-memory fixture, uncontrolled CPU scheduling"})
    );
}
