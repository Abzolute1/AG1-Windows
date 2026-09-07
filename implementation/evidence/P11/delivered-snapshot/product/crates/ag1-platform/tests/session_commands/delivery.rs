use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn poll_delivers_every_operation_event_before_transient_progress() {
    let resolver = Arc::new(Resolver::new(true));
    let f = attach(
        fixture_configured("http://127.0.0.1:1", "disabled", vec![], None, None, 1),
        resolver.clone(),
    );
    let sub = f.commands.subscribe().unwrap();
    let first = accepted(
        f.commands
            .dispatch(prompt(&f.commands, "one", 0, "One"))
            .await
            .result,
    );
    let second = accepted(
        f.commands
            .dispatch(prompt(&f.commands, "two", 0, "Two"))
            .await
            .result,
    );
    let task = {
        let c = f.commands.clone();
        tokio::spawn(async move { c.run_next(&ignore).await })
    };
    resolver.entered.notified().await;
    f.commands
        .progress(&first, "Preparing current turn".into())
        .unwrap();
    assert_eq!(
        f.commands.progress(&second, "Not running".into()),
        Err(w::ProtocolError::NotFound)
    );
    let mut projection = ag1_protocol::projection::Projection::new(sub.snapshot);
    let w::Delivery::Events { events, more, .. } = f.commands.poll(&sub.id).unwrap() else {
        panic!("progress must not swallow unseen admission events")
    };
    assert!(!more);
    assert_eq!(events.len(), 2);
    for event in events {
        assert!(projection.apply(&event).unwrap());
    }
    assert_eq!(projection.snapshot.queued, vec![first.clone(), second]);
    assert!(
        matches!(f.commands.poll(&sub.id).unwrap(), w::Delivery::Progress { text, .. } if text == "Preparing current turn")
    );
    f.commands
        .progress(&first, "Old transient frame".into())
        .unwrap();
    f.host.cancel().unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(f.commands.queued_progress_bytes(&sub.id).unwrap(), 0);
    let w::Delivery::Events { events, .. } = f.commands.poll(&sub.id).unwrap() else {
        panic!()
    };
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].operation.status, w::OperationStatus::Cancelled);
    assert!(projection.apply(&events[0]).unwrap());
    assert_eq!(projection.snapshot, f.commands.snapshot().unwrap());
    assert_eq!(
        f.commands.progress(&first, "Too late".into()),
        Err(w::ProtocolError::NotFound)
    );
    assert!(
        matches!(f.commands.poll(&sub.id).unwrap(), w::Delivery::Events { events, .. } if events.is_empty())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscriber_capacity_and_progress_overflow_do_not_block_native_settlement() {
    let resolver = Arc::new(Resolver::new(true));
    let f = attach(fixture("http://127.0.0.1:1", "disabled"), resolver.clone());
    let mut subscribers = vec![];
    for _ in 0..64 {
        subscribers.push(f.commands.subscribe().unwrap());
    }
    assert!(matches!(
        f.commands.subscribe(),
        Err(w::ProtocolError::Capacity { .. })
    ));
    let operation = accepted(
        f.commands
            .dispatch(prompt(&f.commands, "overflow", 0, "Inspect"))
            .await
            .result,
    );
    let task = {
        let c = f.commands.clone();
        tokio::spawn(async move { c.run_next(&ignore).await })
    };
    resolver.entered.notified().await;
    let fast = &subscribers[0].id;
    f.commands.poll(fast).unwrap();
    for _ in 0..40 {
        f.commands
            .progress(&operation, "x".repeat(32 * 1024))
            .unwrap();
        let delivery = f.commands.poll(fast).unwrap();
        assert!(matches!(&delivery, w::Delivery::Progress { .. }));
        assert!(w::encode(&w::WireMessage::Delivery(delivery)).is_ok());
        for sub in &subscribers {
            assert!(f.commands.queued_progress_bytes(&sub.id).unwrap() <= w::MAX_SUBSCRIBER_BYTES);
        }
    }
    let slow = &subscribers[1].id;
    let w::Delivery::Gap { snapshot } = f.commands.poll(slow).unwrap() else {
        panic!("slow reader must resync")
    };
    assert_eq!(snapshot.queued, vec![operation]);
    assert_eq!(f.commands.queued_progress_bytes(slow).unwrap(), 0);
    f.host.cancel().unwrap();
    let done = task.await.unwrap().unwrap().unwrap();
    assert_eq!(done.status, w::OperationStatus::Cancelled);
    assert!(f.host.settled().unwrap());
    assert!(f.host.persistence_healthy().unwrap());
    assert!(matches!(
        lookup(&f.commands, "overflow").await,
        w::CommandResult::Settled { .. }
    ));
    for sub in subscribers {
        f.commands.unsubscribe(&sub.id).unwrap();
        assert_eq!(f.commands.poll(&sub.id), Err(w::ProtocolError::NotFound));
    }
    assert!(f.commands.subscribe().is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_text_progress_is_delivered_even_when_the_external_listener_panics() {
    let (origin, server) = server(vec![stop()]);
    let f = attach(fixture(&origin, "disabled"), Arc::new(Resolver::new(false)));
    let sub = f.commands.subscribe().unwrap();
    accepted(
        f.commands
            .dispatch(prompt(&f.commands, "stream", 0, "Stream"))
            .await
            .result,
    );
    let observed = Mutex::new(vec![]);
    let listener = |event: &SessionEvent| {
        if matches!(
            event,
            SessionEvent::Provider(ag1_core::provider::ProviderEvent::Delta { .. })
        ) {
            observed
                .lock()
                .unwrap()
                .push(f.commands.poll(&sub.id).unwrap());
            observed
                .lock()
                .unwrap()
                .push(f.commands.poll(&sub.id).unwrap());
            panic!("fixture external client failed after observing progress");
        }
    };
    let result = f.commands.run_next(&listener).await.unwrap().unwrap();
    assert_eq!(result.status, w::OperationStatus::Completed);
    let observed = observed.lock().unwrap();
    assert!(matches!(&observed[0], w::Delivery::Events { events, .. } if events.len() == 1));
    assert!(matches!(&observed[1], w::Delivery::Progress { text, .. } if text == "Done"));
    assert!(
        matches!(f.commands.poll(&sub.id).unwrap(), w::Delivery::Events { events, .. } if events[0].operation.status == w::OperationStatus::Completed)
    );
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversize_transient_delta_becomes_a_gap_without_losing_the_completed_message() {
    let text = "x".repeat(w::MAX_ENVELOPE + 1);
    let reply = json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}]});
    let (origin, server) = server(vec![reply]);
    let f = attach(fixture(&origin, "disabled"), Arc::new(Resolver::new(false)));
    let sub = f.commands.subscribe().unwrap();
    accepted(
        f.commands
            .dispatch(prompt(
                &f.commands,
                "large-progress",
                0,
                "Retain complete answer",
            ))
            .await
            .result,
    );
    let result = f.commands.run_next(&ignore).await.unwrap().unwrap();
    assert_eq!(result.status, w::OperationStatus::Completed);
    let w::Delivery::Gap { snapshot } = f.commands.poll(&sub.id).unwrap() else {
        panic!()
    };
    assert_eq!(snapshot.last_terminal, Some(result));
    assert_eq!(f.commands.queued_progress_bytes(&sub.id).unwrap(), 0);
    let stored = f.store.snapshot("session").unwrap();
    assert!(stored.entries.iter().any(|entry| {
        serde_json::from_slice::<Value>(&entry.normalized).unwrap()["message"]["content"][0]["text"]
            == text
    }));
    assert_eq!(server.join().unwrap().len(), 1);
}
