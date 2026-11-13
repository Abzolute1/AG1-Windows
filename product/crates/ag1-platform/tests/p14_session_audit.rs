#![cfg(target_os = "linux")]
include!("support/session_fixture.rs");
use ag1_core::tools::approval::ApprovalRequest;
use ag1_platform::session::files::ApprovalHandler;
use std::sync::atomic::AtomicU64;

struct ReviewClock(AtomicU64);
impl Clock for ReviewClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_previews_are_journaled_once_and_explicit_deletion_is_only_a_preview() {
    let (endpoint, server) = server(vec![
        calls(vec![
            (
                "missing",
                "editPreview",
                json!({"path":"a.txt","oldText":"original"}),
            ),
            (
                "null",
                "editPreview",
                json!({"path":"a.txt","oldText":"original","newText":null}),
            ),
            (
                "delete",
                "editPreview",
                json!({"path":"a.txt","oldText":"original","newText":""}),
            ),
        ]),
        stop(),
    ]);
    let f = fixture_configured_tools(
        &endpoint,
        "disabled",
        vec![],
        None,
        None,
        2,
        &["editPreview"],
    );
    std::fs::write(f.root.join("a.txt"), "original").unwrap();
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    let records = f.host.client_records().unwrap();
    let results: Vec<_> = records
        .iter()
        .filter(|r| r["message"]["role"] == "toolResult")
        .collect();
    assert_eq!(results.len(), 3);
    for result in &results[..2] {
        assert_eq!(result["message"]["isError"], true);
        assert!(result["message"]["content"].to_string().contains("newText"));
    }
    assert_ne!(results[2]["message"]["isError"], true);
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "original"
    );
    assert_eq!(server.join().unwrap().len(), 2);
}
struct Review {
    clock: Arc<ReviewClock>,
    allow: bool,
    alter: Option<std::path::PathBuf>,
}
impl ApprovalHandler for Review {
    fn approve(&self, request: &ApprovalRequest, _: &AtomicBool) -> Result<bool, DomainError> {
        assert_eq!(request.binding.expires_ms, 301_000);
        self.clock.0.store(300_999, Ordering::SeqCst);
        if let Some(path) = &self.alter {
            std::fs::write(path, "externally changed").unwrap();
        }
        Ok(self.allow)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delayed_native_review_preserves_revision_fences_and_terminal_journaling() {
    for (allow, change) in [(true, false), (false, false), (true, true)] {
        let (endpoint, server) = server(vec![
            calls(vec![(
                "write",
                "write",
                json!({"path":"output.txt","content":"approved"}),
            )]),
            stop(),
        ]);
        let mut f = fixture_seeded(&endpoint, "interactive", vec![]);
        std::fs::write(f.root.join("output.txt"), "original").unwrap();
        let clock = Arc::new(ReviewClock(AtomicU64::new(1000)));
        let runner = Arc::get_mut(&mut f.files).unwrap();
        runner.clock = clock.clone();
        runner.approvals = Arc::new(Review {
            clock,
            allow,
            alter: change.then(|| f.root.join("output.txt")),
        });
        let phases = Mutex::new(Vec::new());
        let outcome = f
            .host
            .run(
                input(),
                &Resolver::new(false),
                f.files.clone(),
                &|event: &SessionEvent| {
                    if let SessionEvent::Recovery(value) = event {
                        if value["type"] == "request_phase" {
                            phases.lock().unwrap().push(value["phase"].clone());
                        }
                    }
                },
            )
            .await;
        assert!(
            outcome.is_ok(),
            "allow={allow}, changed={change}: {:?}; records={:?}",
            outcome.as_ref().err(),
            f.host.client_records()
        );
        assert_eq!(
            std::fs::read_to_string(f.root.join("output.txt")).unwrap(),
            if change {
                "externally changed"
            } else if allow {
                "approved"
            } else {
                "original"
            }
        );
        let records = f.host.client_records().unwrap();
        let results: Vec<_> = records
            .iter()
            .filter(|r| r["message"]["role"] == "toolResult")
            .collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["message"]["isError"], !allow || change);
        assert_eq!(
            *phases.lock().unwrap(),
            vec![
                json!("preparing"),
                json!("waiting"),
                json!("executing"),
                json!("preparing"),
                json!("waiting")
            ]
        );
        assert!(f.host.settled().unwrap());
        assert_eq!(server.join().unwrap().len(), 2);
    }
}
