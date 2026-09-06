use ag1_core::provider::*;
use ag1_core::{DomainError, OperationId, inference::*};
fn op(n: usize) -> OperationId {
    OperationId::new(format!("operation-{n}")).unwrap()
}
fn scheduler(kind: EndpointKind) -> Scheduler {
    let s = Scheduler::default();
    s.register("endpoint", kind).unwrap();
    s
}
#[test]
fn fifo_local_one_remote_three_and_independent_endpoints() {
    for (kind, capacity) in [(EndpointKind::Local, 1), (EndpointKind::Remote, 3)] {
        let s = scheduler(kind);
        s.register("other", EndpointKind::Local).unwrap();
        let tickets: Vec<_> = (0..8)
            .map(|i| s.submit("endpoint", op(i)).unwrap())
            .collect();
        let mut permits = std::collections::VecDeque::new();
        for t in &tickets[..capacity] {
            permits.push_back(s.start(t).unwrap().unwrap());
        }
        assert!(s.start(&tickets[capacity]).unwrap().is_none());
        let other = s.submit("other", op(0)).unwrap();
        s.start(&other).unwrap().unwrap().terminal().unwrap();
        for t in &tickets[capacity..] {
            permits.pop_front().unwrap().terminal().unwrap();
            permits.push_back(s.start(t).unwrap().unwrap());
        }
        for permit in permits {
            permit.terminal().unwrap();
        }
        assert_eq!(s.status("endpoint").unwrap().active, 0);
    }
}
#[test]
fn bounded_admission_cancellation_and_duplicate_isolated_acquisition() {
    let s = scheduler(EndpointKind::Local);
    let t = s.submit("endpoint", op(0)).unwrap();
    let permit = s.start(&t).unwrap().unwrap();
    assert!(matches!(s.start(&t), Err(DomainError::Conflict)));
    assert_eq!(
        s.submit("endpoint", op(0)).unwrap_err(),
        DomainError::Conflict
    );
    let mut queued = Vec::new();
    for i in 1..=128 {
        queued.push(s.submit("endpoint", op(i)).unwrap());
    }
    assert_eq!(
        s.submit("endpoint", op(129)).unwrap_err(),
        DomainError::QueueFull
    );
    s.cancel(&queued[0]).unwrap();
    assert!(matches!(s.start(&queued[0]), Err(DomainError::NotFound)));
    s.submit("endpoint", op(129)).unwrap();
    permit.terminal().unwrap();
    assert!(s.start(&queued[2]).unwrap().is_none());
    s.start(&queued[1]).unwrap().unwrap().terminal().unwrap();
}
#[test]
fn ignored_abort_drop_and_stale_release_never_admit_second_local_generation() {
    let s = scheduler(EndpointKind::Local);
    let first = s.submit("endpoint", op(0)).unwrap();
    let p = s.start(&first).unwrap().unwrap();
    let next = s.submit("endpoint", op(1)).unwrap();
    s.cancel(&first).unwrap();
    for _ in 0..100 {
        assert!(s.start(&next).unwrap().is_none());
    }
    assert_eq!(s.status("endpoint").unwrap().quarantined, 1);
    drop(p);
    assert!(s.start(&next).unwrap().is_none());
    s.reconcile(&first).unwrap();
    let next_p = s.start(&next).unwrap().unwrap();
    assert_eq!(s.reconcile(&first), Err(DomainError::NotFound));
    assert_eq!(s.status("endpoint").unwrap().active, 1);
    next_p.terminal().unwrap();
}
#[test]
fn cancellation_freezes_already_granted_remote_starts_and_cannot_cross_owners() {
    let s = scheduler(EndpointKind::Remote);
    let foreign = scheduler(EndpointKind::Remote);
    let t = s.submit("endpoint", op(0)).unwrap();
    let f = foreign.submit("endpoint", op(0)).unwrap();
    assert_eq!(s.cancel(&f), Err(DomainError::PermissionDenied));
    assert!(matches!(s.start(&f), Err(DomainError::PermissionDenied)));
    assert_eq!(s.reconcile(&f), Err(DomainError::PermissionDenied));
    let p = s.start(&t).unwrap().unwrap();
    let waiting = s.submit("endpoint", op(1)).unwrap();
    s.cancel(&t).unwrap();
    assert!(s.start(&waiting).unwrap().is_none());
    p.terminal().unwrap();
    s.start(&waiting).unwrap().unwrap().terminal().unwrap();
}
#[test]
fn concurrent_admission_retains_single_owner_and_queue_bounds() {
    let s = scheduler(EndpointKind::Local);
    let joins: Vec<_> = (0..64)
        .map(|i| {
            let s = s.clone();
            std::thread::spawn(move || {
                let t = s.submit("endpoint", op(i)).unwrap();
                let p = s.start(&t).unwrap();
                (t, p)
            })
        })
        .collect();
    let values: Vec<_> = joins.into_iter().map(|j| j.join().unwrap()).collect();
    assert_eq!(values.iter().filter(|(_, p)| p.is_some()).count(), 1);
    assert_eq!(s.status("endpoint").unwrap().waiting, 63);
    drop(values);
    assert_eq!(s.status("endpoint").unwrap().quarantined, 1);
}

fn begin_tool() -> ProviderEvent {
    ProviderEvent::Begin {
        key: "tool".into(),
        kind: BlockKind::Tool,
        id: "call".into(),
        name: "read".into(),
        arguments: None,
        redacted: false,
    }
}
#[test]
fn partial_tools_and_snapshots_are_stable_and_malformed_calls_never_become_valid() {
    for arguments in ["{\"path\":", "[]", "null", "{oops}", ""] {
        let mut reducer = StreamReducer::new(ApiFamily::Completions, 0, 4096).unwrap();
        reducer.apply(begin_tool(), 0).unwrap();
        reducer
            .apply(
                ProviderEvent::Delta {
                    key: "tool".into(),
                    text: arguments.into(),
                },
                1,
            )
            .unwrap();
        let before = reducer.snapshot();
        assert!(before.content[0].arguments.is_none());
        reducer
            .apply(ProviderEvent::Stop(StopReason::ToolUse), 2)
            .unwrap();
        let final_value = reducer.finish(false).unwrap();
        assert!(final_value.content[0].malformed_arguments);
        assert!(!before.settled);
        assert_eq!(before.content[0].text, arguments);
    }
    let mut reducer = StreamReducer::new(ApiFamily::Responses, 0, 4096).unwrap();
    reducer.apply(begin_tool(), 0).unwrap();
    reducer
        .apply(
            ProviderEvent::Delta {
                key: "tool".into(),
                text: "{\"path\":".into(),
            },
            1,
        )
        .unwrap();
    let snapshot = reducer.snapshot();
    reducer
        .apply(
            ProviderEvent::Delta {
                key: "tool".into(),
                text: "\"a.txt\"}".into(),
            },
            2,
        )
        .unwrap();
    reducer
        .apply(ProviderEvent::Stop(StopReason::Stop), 3)
        .unwrap();
    let result = reducer.finish(false).unwrap();
    assert_eq!(result.stop_reason, Some(StopReason::ToolUse));
    assert!(!result.content[0].malformed_arguments);
    assert_eq!(snapshot.content[0].text, "{\"path\":");
    assert!(snapshot.content[0].arguments.is_none());
}
#[test]
fn slow_prefill_watchdog_cancellation_and_eof_do_not_fabricate_success() {
    let mut reducer = StreamReducer::new(ApiFamily::Completions, 100, 4096).unwrap();
    assert_eq!(reducer.watchdog(60_000, 180_000, 5000), Ok(()));
    assert_eq!(
        reducer.watchdog(180_100, 180_000, 5000),
        Err(DomainError::TimedOut)
    );
    reducer
        .apply(
            ProviderEvent::Begin {
                key: "text".into(),
                kind: BlockKind::Text,
                id: String::new(),
                name: String::new(),
                arguments: None,
                redacted: false,
            },
            100,
        )
        .unwrap();
    reducer
        .apply(
            ProviderEvent::Delta {
                key: "text".into(),
                text: "hello".into(),
            },
            120,
        )
        .unwrap();
    assert_eq!(reducer.watchdog(5119, 180000, 5000), Ok(()));
    assert_eq!(
        reducer.watchdog(5120, 180000, 5000),
        Err(DomainError::TimedOut)
    );
    assert!(!reducer.snapshot().settled);
    assert_eq!(
        reducer.finish(false).unwrap().stop_reason,
        Some(StopReason::Error)
    );
    let mut reducer = StreamReducer::new(ApiFamily::Completions, 0, 4096).unwrap();
    reducer
        .apply(ProviderEvent::Stop(StopReason::Stop), 0)
        .unwrap();
    assert_eq!(
        reducer.finish(true).unwrap().stop_reason,
        Some(StopReason::Aborted)
    );
}
#[test]
fn provider_bounds_usage_overflow_and_error_precedence_are_explicit() {
    let mut reducer = StreamReducer::new(ApiFamily::Anthropic, 0, 1024).unwrap();
    reducer.apply(begin_tool(), 0).unwrap();
    assert!(matches!(
        reducer.apply(
            ProviderEvent::Delta {
                key: "tool".into(),
                text: "x".repeat(1024)
            },
            1
        ),
        Err(DomainError::LimitExceeded(_))
    ));
    reducer
        .apply(ProviderEvent::Stop(StopReason::Stop), 2)
        .unwrap();
    assert_eq!(
        reducer.finish(false).unwrap().stop_reason,
        Some(StopReason::Error)
    );
    let mut reducer = StreamReducer::new(ApiFamily::Anthropic, 0, 1024).unwrap();
    assert!(
        reducer
            .apply(
                ProviderEvent::Usage(UsagePatch {
                    input: Some(u64::MAX),
                    output: Some(1),
                    ..Default::default()
                }),
                0
            )
            .is_err()
    );
    assert!(Usage::default().cost([f64::NAN, 0.0, 0.0, 0.0]).is_err());
    assert!(Usage::default().cost([-1.0, 0.0, 0.0, 0.0]).is_err());
}

#[test]
fn nvidia_fallback_only_precedes_generation_and_never_changes_preserved_llama() {
    use ag1_core::fallback::*;
    fn route(provider: &str, model: &str, auth: bool) -> ModelRoute {
        ModelRoute {
            provider: provider.into(),
            model: model.into(),
            endpoint: "https://fixture.invalid".into(),
            configured_auth: auth,
        }
    }
    let primary = route("nvidia", "primary", true);
    let cloud = route("openrouter", "nvidia/nemotron-3-ultra-550b-a55b:free", true);
    let local = route("local", "local-model", true);
    let mut plan =
        FallbackSelection::new(primary.clone(), &[cloud.clone(), local.clone()]).unwrap();
    assert!(matches!(
        plan.resolved_auth(true).unwrap(),
        SelectionStep::ProbeNvidia(_)
    ));
    assert_eq!(
        plan.preflight_status(403).unwrap(),
        SelectionStep::ResolveAuth(cloud.clone())
    );
    assert_eq!(
        plan.resolved_auth(true).unwrap(),
        SelectionStep::Generate(cloud)
    );
    assert_eq!(plan.preflight_status(403), Err(DomainError::Conflict));
    assert_eq!(plan.resolved_auth(false), Err(DomainError::Conflict));
    for status in [400, 401, 429, 500] {
        let mut plan =
            FallbackSelection::new(primary.clone(), std::slice::from_ref(&local)).unwrap();
        plan.resolved_auth(true).unwrap();
        assert!(matches!(
            plan.preflight_status(status),
            Err(DomainError::Unavailable(_))
        ));
    }
    let mut plan = FallbackSelection::new(
        primary.clone(),
        &[route("local", "unconfigured-first", false), local.clone()],
    )
    .unwrap();
    plan.resolved_auth(true).unwrap();
    assert!(plan.preflight_status(403).is_err());
    for provider in ["llamacpp", "llama.cpp", "anthropic", "local"] {
        let model = route(provider, "selected", true);
        let mut plan = FallbackSelection::new(model.clone(), std::slice::from_ref(&local)).unwrap();
        assert_eq!(
            plan.resolved_auth(true).unwrap(),
            SelectionStep::Generate(model)
        );
        assert_eq!(plan.preflight_status(403), Err(DomainError::Conflict));
    }
}
#[test]
fn isolated_context_has_no_root_tools_and_incomplete_memory_is_rejected() {
    use ag1_core::isolated::*;
    let context = context(IsolatedInput {
        system: " system ",
        prompt: " prompt ",
        purpose: "Episode / Summary",
        requested_max: Some(99999),
        model_max: 4096,
        root_session: Some("root"),
    })
    .unwrap();
    assert_eq!(context.max_tokens, 16384);
    assert_eq!(
        context.session_id.as_deref(),
        Some("root:isolated:episode-summary")
    );
    assert!(context.context.get("tools").is_none());
    assert_eq!(context.context["messages"].as_array().unwrap().len(), 1);
    let mut reducer = StreamReducer::new(ApiFamily::Completions, 0, 4096).unwrap();
    reducer
        .apply(
            ProviderEvent::Begin {
                key: "text".into(),
                kind: BlockKind::Text,
                id: String::new(),
                name: String::new(),
                arguments: None,
                redacted: false,
            },
            0,
        )
        .unwrap();
    reducer
        .apply(
            ProviderEvent::Delta {
                key: "text".into(),
                text: " partial memory ".into(),
            },
            1,
        )
        .unwrap();
    reducer
        .apply(ProviderEvent::Stop(StopReason::Length), 2)
        .unwrap();
    let snapshot = reducer.finish(false).unwrap();
    assert!(accepted_text(&snapshot, false).is_err());
    assert_eq!(accepted_text(&snapshot, true).unwrap(), "partial memory");
}
