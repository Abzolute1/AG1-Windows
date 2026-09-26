use super::*;
use ag1_core::tools::{jobs::Notice, registry::Scope};
use ag1_platform::session::{ToolOutcome, ToolRunner, ToolSelection, files::NativeToolRunner};

fn start() -> Value {
    calls(vec![(
        "start",
        "job",
        json!({"action":"start","command":"printf 'once\\n' >> executions.txt; printf 'job output\\n'"}),
    )])
}
fn next(command: &str) -> TurnInput {
    let mut next = input();
    next.command = command.into();
    next
}
async fn completed_job(fixture: &Fixture, runner: Arc<NativeToolRunner>) -> Notice {
    fixture
        .host
        .run(input(), &Resolver::new(false), runner.clone(), &ignore)
        .await
        .unwrap();
    let jobs = runner.adapters.jobs.as_ref().unwrap();
    let mut artifacts = ag1_storage::context::ContextStore::new(
        &fixture.store,
        SessionId::new("session").unwrap(),
        "session",
        "notice-observer",
        NOW,
        Default::default(),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            jobs.refresh(&mut artifacts).unwrap();
            if let Some(notice) = jobs.notices().unwrap().into_iter().next() {
                break notice;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn parent_notices(fixture: &Fixture) -> Vec<Value> {
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    ag1_core::task::branch_values(&journal)
        .unwrap()
        .into_iter()
        .filter(|entry| entry["message"]["customType"] == "background_job_notice")
        .collect()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_job_reaches_the_next_prompt_once_after_parent_custody() {
    let (origin, server) = server(vec![start(), stop(), stop(), stop()]);
    let fixture = fixture(&origin, "disabled");
    let runner = native_adapters(&fixture, &["job"], None).await;
    let notice = completed_job(&fixture, runner.clone()).await;
    fixture
        .host
        .run(
            next("second"),
            &Resolver::new(false),
            runner.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert!(
        runner
            .adapters
            .jobs
            .as_ref()
            .unwrap()
            .notices()
            .unwrap()
            .is_empty()
    );
    fixture
        .host
        .run(next("third"), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    let messages = parent_notices(&fixture);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["message"]["content"], notice.text);
    assert_eq!(messages[0]["message"]["details"]["noticeId"], notice.id);
    let requests = server.join().unwrap();
    assert!(
        requests[2]["messages"]
            .to_string()
            .contains("Background jobs finished:")
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("executions.txt")).unwrap(),
        "once\n"
    );
    assert!(fixture.host.settled().unwrap());
}

struct NoticeFault {
    point: ag1_storage::Boundary,
    metadata: bool,
    armed: AtomicBool,
}
impl ag1_storage::FaultInjector for NoticeFault {
    fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if self.metadata && self.point == point && self.armed.swap(false, Ordering::AcqRel) {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        point: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if !self.metadata
            && command.starts_with("job-notice-")
            && self.point == point
            && self.armed.swap(false, Ordering::AcqRel)
        {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_notice_parent_commit_or_ack_failure_never_marks_notification_delivered() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, server) = server(vec![start(), stop()]);
        let fault = Arc::new(NoticeFault {
            point,
            metadata: false,
            armed: AtomicBool::new(true),
        });
        let fixture = fixture_seeded_with_fault(&origin, "disabled", vec![], Some(fault));
        let runner = native_adapters(&fixture, &["job"], None).await;
        completed_job(&fixture, runner.clone()).await;
        assert!(
            fixture
                .host
                .run(
                    next("second"),
                    &Resolver::new(false),
                    runner.clone(),
                    &ignore
                )
                .await
                .is_err()
        );
        assert!(!fixture.host.persistence_healthy().unwrap());
        assert_eq!(
            runner
                .adapters
                .jobs
                .as_ref()
                .unwrap()
                .notices()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parent_notices(&fixture).len(),
            usize::from(point == ag1_storage::Boundary::BeforeAck)
        );
        assert_eq!(server.join().unwrap().len(), 2);
    }
}
#[derive(Default)]
struct AckGate {
    entered: Notify,
    released: Mutex<bool>,
    changed: std::sync::Condvar,
}
impl AckGate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
    fn wait(&self) {
        self.entered.notify_one();
        let (released, _) = self
            .changed
            .wait_timeout_while(
                self.released.lock().unwrap(),
                Duration::from_secs(5),
                |released| !*released,
            )
            .unwrap();
        assert!(*released, "release job notice acknowledgement");
    }
}
struct NoticeRunner {
    inner: Arc<NativeToolRunner>,
    fault: Option<Arc<NoticeFault>>,
    gate: Option<Arc<AckGate>>,
    acknowledgements: AtomicUsize,
}
impl ToolRunner for NoticeRunner {
    fn execute(
        &self,
        scope: &Scope,
        calls: &[ag1_core::runtime::session::PendingCall],
        selection: &ToolSelection,
        operation: &str,
        cancel: &AtomicBool,
        revision: &std::sync::atomic::AtomicU64,
    ) -> Result<Vec<ToolOutcome>, DomainError> {
        self.inner
            .execute(scope, calls, selection, operation, cancel, revision)
    }
    fn quiesce(&self, scope: &Scope, operation: &str) -> Result<(), DomainError> {
        self.inner.quiesce(scope, operation)
    }
    fn job_notices(&self, scope: &Scope, operation: &str) -> Result<Vec<Notice>, DomainError> {
        self.inner.job_notices(scope, operation)
    }
    fn acknowledge_job_notice(&self, scope: &Scope, id: &str) -> Result<(), DomainError> {
        if self.acknowledgements.fetch_add(1, Ordering::AcqRel) == 0 {
            if let Some(fault) = &self.fault {
                fault.armed.store(true, Ordering::Release);
            }
            if let Some(gate) = &self.gate {
                gate.wait();
            }
        }
        self.inner.acknowledge_job_notice(scope, id)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ambiguous_job_metadata_ack_retries_without_duplicate_parent_message_or_process() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, server) = server(vec![start(), stop(), stop()]);
        let fault = Arc::new(NoticeFault {
            point,
            metadata: true,
            armed: AtomicBool::new(false),
        });
        let fixture = fixture_seeded_with_fault(&origin, "disabled", vec![], Some(fault.clone()));
        let inner = native_adapters(&fixture, &["job"], None).await;
        completed_job(&fixture, inner.clone()).await;
        let runner = Arc::new(NoticeRunner {
            inner,
            fault: Some(fault),
            gate: None,
            acknowledgements: AtomicUsize::new(0),
        });
        assert!(
            fixture
                .host
                .run(
                    next("second"),
                    &Resolver::new(false),
                    runner.clone(),
                    &ignore
                )
                .await
                .is_err()
        );
        assert_eq!(parent_notices(&fixture).len(), 1);
        assert!(fixture.host.persistence_healthy().unwrap());
        fixture
            .host
            .run(
                next("third"),
                &Resolver::new(false),
                runner.clone(),
                &ignore,
            )
            .await
            .unwrap();
        assert_eq!(parent_notices(&fixture).len(), 1);
        assert!(
            runner
                .inner
                .adapters
                .jobs
                .as_ref()
                .unwrap()
                .notices()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("executions.txt")).unwrap(),
            "once\n"
        );
        assert_eq!(server.join().unwrap().len(), 3);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_notice_caller_retains_worker_settlement_and_blocks_another_prompt() {
    let (origin, server) = server(vec![start(), stop(), stop()]);
    let fixture = fixture(&origin, "disabled");
    let inner = native_adapters(&fixture, &["job"], None).await;
    completed_job(&fixture, inner.clone()).await;
    let gate = Arc::new(AckGate::default());
    let runner = Arc::new(NoticeRunner {
        inner,
        fault: None,
        gate: Some(gate.clone()),
        acknowledgements: AtomicUsize::new(0),
    });
    let host = fixture.host.clone();
    let worker = runner.clone();
    let run = tokio::spawn(async move {
        host.run(next("second"), &Resolver::new(false), worker, &ignore)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    run.abort();
    assert!(matches!(run.await, Err(error) if error.is_cancelled()));
    assert!(!fixture.host.settled().unwrap());
    assert!(matches!(
        fixture
            .host
            .run(
                next("blocked"),
                &Resolver::new(false),
                runner.clone(),
                &ignore
            )
            .await,
        Err(DomainError::Unavailable(_))
    ));
    assert!(fixture.store.command_result("blocked").unwrap().is_none());
    gate.release();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fixture.host.settled().unwrap() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    fixture
        .host
        .run(
            next("third"),
            &Resolver::new(false),
            runner.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert_eq!(parent_notices(&fixture).len(), 1);
    assert!(
        runner
            .inner
            .adapters
            .jobs
            .as_ref()
            .unwrap()
            .notices()
            .unwrap()
            .is_empty()
    );
    assert_eq!(server.join().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compaction_preserves_owned_job_control_and_later_job_retirement() {
    let long_job = calls(vec![(
        "before-compaction",
        "job",
        json!({"action":"start","command":"printf ready > job-ready; exec /usr/bin/tail -f /dev/null"}),
    )]);
    let later_job = calls(vec![(
        "after-compaction",
        "job",
        json!({"action":"start","command":"printf ready > later-ready; exec /usr/bin/tail -f /dev/null"}),
    )]);
    let (origin, server) = server(vec![
        long_job,
        stop(),
        stop(),
        stop(),
        json!({"fixtureJobAction":"kill"}),
        stop(),
        later_job,
        stop(),
    ]);
    let fixture = fixture_seeded(&origin, "disabled", summary_history());
    let runner = native_adapters(&fixture, &["job"], None).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner.clone(), &ignore)
        .await
        .unwrap();
    let result = session_tool_results(&fixture);
    let id = result
        .iter()
        .find(|message| message["toolCallId"] == "before-compaction")
        .unwrap()["details"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let compacted = fixture
        .host
        .compact(compact_input(), &Resolver::new(false), &ignore)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(compacted.context_epoch, 2);
    let mut kill = next("kill-after-compaction");
    kill.message["content"] = json!(format!("Stop the owned background job {id}."));
    fixture
        .host
        .run(kill, &Resolver::new(false), runner.clone(), &ignore)
        .await
        .unwrap();
    fixture
        .host
        .run(
            next("new-job-after-compaction"),
            &Resolver::new(false),
            runner.clone(),
            &ignore,
        )
        .await
        .unwrap();
    let scope = Scope {
        workspace: fixture.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 2,
        revision: fixture.host.journal_revision().unwrap(),
    };
    let retired = runner.quiesce(&scope, "retire-after-compaction");
    assert_eq!(server.join().unwrap().len(), 8);
    let results = session_tool_results(&fixture);
    let kill = results
        .iter()
        .find(|message| message["toolCallId"] == "job-log")
        .unwrap();
    assert_ne!(kill["isError"], true, "{kill}");
    assert_eq!(kill["details"]["status"], "exited");
    let start = results
        .iter()
        .find(|message| message["toolCallId"] == "after-compaction")
        .unwrap();
    assert_ne!(start["isError"], true, "{start}");
    retired.unwrap();
    assert!(
        std::fs::read_dir(fixture._directory.path().join("tool-spools"))
            .unwrap()
            .next()
            .is_none()
    );
}
