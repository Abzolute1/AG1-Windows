#![cfg(target_os = "linux")]
use ag1_core::DomainError;
use ag1_platform::{
    mcp::{
        Revision,
        catalog::{CatalogBinding, CatalogScope},
        stdio::{CatalogKind, DiscoveryLimits, Health, StdioDiscovery},
    },
    process::{ProcessSpec, ProcessSupervisor},
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    client: StdioDiscovery,
    directory: tempfile::TempDir,
    helper_pid: u32,
}
fn fixture(mode: &str, revision: Revision, timeout: Duration) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("fixture.py");
    std::fs::write(&script, include_str!("fixtures/mcp_stdio.py")).unwrap();
    let supervisor =
        ProcessSupervisor::new(Path::new(env!("CARGO_BIN_EXE_ag1-process-host"))).unwrap();
    let process = supervisor
        .spawn_mcp_protocol(
            ProcessSpec {
                executable: "/usr/bin/python3".into(),
                argv: vec![
                    "-I".into(),
                    script.to_str().unwrap().into(),
                    mode.into(),
                    revision.as_str().into(),
                ],
                cwd: directory.path().into(),
                env: BTreeMap::new(),
                timeout_ms: 10_000,
                grace_ms: 50,
                resident: false,
            },
            &directory.path().join("stderr.log"),
        )
        .unwrap();
    let helper_pid = process.identity().pid;
    let binding = CatalogBinding {
        server_id: "26fb3347-af67-4a1d-92ec-a915db39c5d0".into(),
        scope: CatalogScope::GeneralChat,
        config_revision: 1,
        auth_profile: "owned-fixture".into(),
        credential_generation: 0,
        connection_generation: 1,
        protocol: revision,
    };
    let client =
        StdioDiscovery::from_owned(process, binding, DiscoveryLimits::new(timeout).unwrap())
            .unwrap();
    Fixture {
        client,
        directory,
        helper_pid,
    }
}
fn events(f: &Fixture) -> Vec<String> {
    std::fs::read_to_string(f.directory.path().join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn gone(pid: u32) {
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "owned pid {pid} was not reaped"
    );
}

#[test]
fn both_lifecycles_discover_all_capability_kinds_without_invocation() {
    for revision in [Revision::Legacy20251125, Revision::Current20260728] {
        let mut f = fixture("normal", revision, Duration::from_secs(3));
        let cancel = AtomicBool::new(false);
        f.client.negotiate(&cancel).unwrap();
        assert_eq!(f.client.health(), Health::Ready);
        f.client.refresh_tools(&cancel).unwrap();
        let tools = f.client.tools().unwrap();
        assert_eq!(tools.tools().count(), 2);
        assert_eq!(tools.binding().scope, CatalogScope::GeneralChat);
        assert!(tools.get("first").is_some());
        for kind in [
            CatalogKind::Resources,
            CatalogKind::ResourceTemplates,
            CatalogKind::Prompts,
        ] {
            let catalog = f.client.list(kind, &cancel).unwrap();
            assert_eq!(catalog.entries.len(), 1);
            assert_eq!(catalog.kind, kind);
            assert_eq!(catalog.binding.protocol, revision);
        }
        f.client.shutdown().unwrap();
        assert_eq!(f.client.health(), Health::Stopped);
        assert!(f.client.tools().is_none());
        gone(f.helper_pid);
        let methods = events(&f);
        assert_eq!(
            methods.contains(&"notifications/initialized".into()),
            revision == Revision::Legacy20251125
        );
        assert_eq!(
            methods
                .iter()
                .filter(|m| m.as_str() == "tools/list")
                .count(),
            2
        );
        assert!(methods.iter().all(|m| {
            [
                "initialize",
                "server/discover",
                "notifications/initialized",
                "tools/list",
                "resources/list",
                "resources/templates/list",
                "prompts/list",
            ]
            .contains(&m.as_str())
        }));
    }
}

#[test]
fn unsupported_negotiation_retires_and_reaps_without_fallback() {
    for revision in [Revision::Legacy20251125, Revision::Current20260728] {
        let mut f = fixture("wrong_revision", revision, Duration::from_secs(3));
        assert!(f.client.negotiate(&AtomicBool::new(false)).is_err());
        assert_eq!(f.client.health(), Health::Failed);
        assert!(f.client.server_metadata().is_none());
        assert_eq!(events(&f).len(), 1);
        gone(f.helper_pid);
    }
}

#[test]
fn malformed_oversized_truncated_and_foreign_responses_retire_the_process() {
    for mode in ["oversized", "truncated", "bad_id", "error"] {
        let mut f = fixture(mode, Revision::Current20260728, Duration::from_secs(3));
        let error = f.client.negotiate(&AtomicBool::new(false)).unwrap_err();
        assert!(!format!("{error:?}").contains("synthetic-mcp-secret"));
        assert_eq!(f.client.health(), Health::Failed);
        gone(f.helper_pid);
        assert!(f.client.negotiate(&AtomicBool::new(false)).is_err());
        assert_eq!(events(&f).len(), 1);
    }
}

#[test]
fn capability_changes_floods_duplicate_ids_and_continuations_never_publish() {
    for mode in ["changed", "flood", "duplicate_response", "continuation"] {
        let mut f = fixture(mode, Revision::Current20260728, Duration::from_secs(3));
        let cancel = AtomicBool::new(false);
        f.client.negotiate(&cancel).unwrap();
        let error = f.client.refresh_tools(&cancel).unwrap_err();
        assert!(!format!("{error:?}").contains("opaque-private-state"));
        assert!(f.client.tools().is_none());
        assert_eq!(f.client.health(), Health::Failed);
        gone(f.helper_pid);
    }
}

#[test]
fn unadvertised_capabilities_are_not_queried() {
    let mut f = fixture(
        "no_capabilities",
        Revision::Current20260728,
        Duration::from_secs(3),
    );
    let cancel = AtomicBool::new(false);
    f.client.negotiate(&cancel).unwrap();
    assert!(f.client.refresh_tools(&cancel).is_err());
    assert!(f.client.list(CatalogKind::Prompts, &cancel).is_err());
    assert_eq!(f.client.health(), Health::Ready);
    f.client.shutdown().unwrap();
    assert_eq!(events(&f), vec!["server/discover"]);
}

#[test]
fn unsolicited_sampling_gets_explicit_refusal_and_discovery_can_finish() {
    let mut f = fixture(
        "server_request",
        Revision::Legacy20251125,
        Duration::from_secs(3),
    );
    let cancel = AtomicBool::new(false);
    f.client.negotiate(&cancel).unwrap();
    f.client.refresh_tools(&cancel).unwrap();
    f.client.shutdown().unwrap();
    assert_eq!(
        events(&f)
            .iter()
            .filter(|m| *m == "sampling_refused")
            .count(),
        2
    );
    gone(f.helper_pid);
}

#[test]
fn current_revision_rejects_standalone_server_requests() {
    let mut f = fixture(
        "server_request",
        Revision::Current20260728,
        Duration::from_secs(3),
    );
    let cancel = AtomicBool::new(false);
    f.client.negotiate(&cancel).unwrap();
    assert!(f.client.refresh_tools(&cancel).is_err());
    assert_eq!(f.client.health(), Health::Failed);
    gone(f.helper_pid);
    assert!(!events(&f).contains(&"sampling_refused".into()));
}

#[test]
fn timeout_is_bounded_and_never_replays_discovery() {
    let mut f = fixture(
        "hang",
        Revision::Current20260728,
        Duration::from_millis(150),
    );
    let start = Instant::now();
    assert!(matches!(
        f.client.negotiate(&AtomicBool::new(false)),
        Err(DomainError::TimedOut)
    ));
    assert!(start.elapsed() < Duration::from_secs(4));
    assert_eq!(events(&f), vec!["server/discover"]);
    gone(f.helper_pid);
}

#[test]
fn cancellation_before_send_and_during_wait_reaps_owned_process() {
    let mut before = fixture("normal", Revision::Current20260728, Duration::from_secs(3));
    assert!(matches!(
        before.client.negotiate(&AtomicBool::new(true)),
        Err(DomainError::Cancelled)
    ));
    assert!(events(&before).is_empty());
    gone(before.helper_pid);
    let mut during = fixture("hang", Revision::Current20260728, Duration::from_secs(3));
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let sender = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        signal.store(true, Ordering::Release);
    });
    assert!(matches!(
        during.client.negotiate(&cancel),
        Err(DomainError::Cancelled)
    ));
    sender.join().unwrap();
    gone(during.helper_pid);
}

#[test]
fn shutdown_and_drop_reap_descendants_even_after_they_create_a_session() {
    for explicit in [false, true] {
        let mut f = fixture(
            "descendant",
            Revision::Current20260728,
            Duration::from_secs(3),
        );
        f.client.negotiate(&AtomicBool::new(false)).unwrap();
        let child: u32 = std::fs::read_to_string(f.directory.path().join("descendant.pid"))
            .unwrap()
            .parse()
            .unwrap();
        if explicit {
            f.client.shutdown().unwrap();
        }
        let helper = f.helper_pid;
        drop(f);
        gone(helper);
        gone(child);
    }
}

#[test]
fn stderr_is_discarded_at_capture_and_never_enters_the_spool() {
    let mut f = fixture("stderr", Revision::Current20260728, Duration::from_secs(3));
    f.client.negotiate(&AtomicBool::new(false)).unwrap();
    f.client.shutdown().unwrap();
    assert!(
        std::fs::read(f.directory.path().join("stderr.log"))
            .unwrap()
            .is_empty()
    );
    gone(f.helper_pid);
}

#[test]
fn excessive_lifetime_is_rejected_before_launch() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor =
        ProcessSupervisor::new(Path::new(env!("CARGO_BIN_EXE_ag1-process-host"))).unwrap();
    let result = supervisor.spawn_mcp_protocol(
        ProcessSpec {
            executable: "/usr/bin/python3".into(),
            argv: vec![],
            cwd: dir.path().into(),
            env: BTreeMap::new(),
            timeout_ms: 600_001,
            grace_ms: 50,
            resident: false,
        },
        &dir.path().join("must-not-exist.log"),
    );
    assert!(result.is_err());
    assert!(!dir.path().join("must-not-exist.log").exists());
}

#[test]
fn resource_pages_are_complete_and_hostile_catalogs_are_refused() {
    let cancel = AtomicBool::new(false);
    let mut good = fixture(
        "catalog_pages",
        Revision::Current20260728,
        Duration::from_secs(3),
    );
    good.client.negotiate(&cancel).unwrap();
    assert_eq!(
        good.client
            .list(CatalogKind::Resources, &cancel)
            .unwrap()
            .entries
            .len(),
        2
    );
    good.client.shutdown().unwrap();
    for mode in [
        "catalog_cursor",
        "catalog_changed",
        "catalog_huge",
        "catalog_duplicate",
    ] {
        let mut f = fixture(mode, Revision::Current20260728, Duration::from_secs(3));
        f.client.negotiate(&cancel).unwrap();
        let kind = if mode == "catalog_duplicate" {
            CatalogKind::Prompts
        } else {
            CatalogKind::Resources
        };
        assert!(f.client.list(kind, &cancel).is_err());
        assert_eq!(f.client.health(), Health::Failed);
        gone(f.helper_pid);
    }
}

#[test]
fn discovery_deadline_covers_the_entire_paginated_operation() {
    let mut f = fixture(
        "slow_pages",
        Revision::Current20260728,
        Duration::from_millis(150),
    );
    let cancel = AtomicBool::new(false);
    f.client.negotiate(&cancel).unwrap();
    assert!(matches!(
        f.client.refresh_tools(&cancel),
        Err(DomainError::TimedOut)
    ));
    assert!(f.client.tools().is_none());
    gone(f.helper_pid);
}

#[test]
fn cancelling_one_owned_connection_does_not_stop_another() {
    let mut first = fixture("normal", Revision::Current20260728, Duration::from_secs(3));
    let mut second = fixture("normal", Revision::Current20260728, Duration::from_secs(3));
    assert!(matches!(
        first.client.negotiate(&AtomicBool::new(true)),
        Err(DomainError::Cancelled)
    ));
    gone(first.helper_pid);
    let cancel = AtomicBool::new(false);
    second.client.negotiate(&cancel).unwrap();
    second.client.refresh_tools(&cancel).unwrap();
    assert_eq!(second.client.tools().unwrap().tools().count(), 2);
    second.client.shutdown().unwrap();
    gone(second.helper_pid);
}
