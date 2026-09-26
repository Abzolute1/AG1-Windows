use ag1_core::{
    DomainError, PortFuture,
    resources::{
        package_lifecycle::Scope,
        package_queries::{self, Port, Query},
    },
};
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};
fn run<F: Future>(future: F) -> F::Output {
    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}
struct Fixture {
    input: Value,
    calls: Mutex<Vec<Value>>,
}
impl Port for Fixture {
    fn checkpoint(&self) -> Result<(), DomainError> {
        Ok(())
    }
    fn exists<'a>(&'a self, _: &'a str) -> PortFuture<'a, bool> {
        Box::pin(async { Ok(true) })
    }
    fn manifest<'a>(&'a self, _: &'a str) -> PortFuture<'a, Option<Value>> {
        Box::pin(async {
            Ok(Some(
                self.input
                    .get("installed")
                    .map(|v| json!({"version":v}))
                    .unwrap_or(json!({})),
            ))
        })
    }
    fn latest_npm<'a>(&'a self, name: &'a str, timeout: u64) -> PortFuture<'a, Option<Value>> {
        Box::pin(async move {
            assert_eq!(timeout, 10_000);
            self.calls.lock().unwrap().push(json!({"kind":"registry","url":format!("https://registry.npmjs.org/{name}/latest"),"signal":true}));
            if self.input["fail"] == true || self.input["badStatus"] == true {
                return Err(DomainError::Unavailable("fixture".into()));
            }
            Ok(self.input.get("latest").cloned())
        })
    }
    fn git<'a>(
        &'a self,
        _: &'a str,
        args: Vec<String>,
        remote: bool,
        timeout: u64,
    ) -> PortFuture<'a, String> {
        Box::pin(async move {
            let mut options = json!({"cwd":"/fixture/installed","timeoutMs":timeout});
            if remote {
                options["env"] = json!({"GIT_TERMINAL_PROMPT":"0"});
            }
            self.calls
                .lock()
                .unwrap()
                .push(json!({"kind":"git","command":"git","args":args,"options":options}));
            if args == ["rev-parse", "HEAD"] {
                if self.input["failLocal"] == true {
                    return Err(DomainError::Unavailable("fixture".into()));
                }
                return Ok(format!("{}\n", "a".repeat(40)));
            }
            if args[0] == "rev-parse" {
                if self.input["noUpstream"] == true {
                    return Err(DomainError::Unavailable("fixture".into()));
                }
                return Ok(self.input["upstream"]
                    .as_str()
                    .unwrap_or("origin/main\n")
                    .into());
            }
            if self.input["failRemote"] == true {
                return Err(DomainError::Unavailable("fixture".into()));
            }
            if args[2] != "HEAD" && self.input["emptyUpstreamHead"] == true {
                return Ok(String::new());
            }
            if self.input["badHead"] == true {
                return Ok("invalid\tHEAD".into());
            }
            Ok(format!(
                "{}\t{}\n",
                if self.input["same"] == true { "a" } else { "b" }.repeat(40),
                args[2]
            ))
        })
    }
}
#[test]
fn native_version_and_git_queries_match_all_sdk_outputs_and_port_calls() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/package-queries-oracle.json"
    ))
    .unwrap();
    for row in oracle["rows"].as_array().unwrap() {
        let fixture = Fixture {
            input: row["input"].clone(),
            calls: Mutex::new(Vec::new()),
        };
        let source = if fixture.input["kind"] == "npm" {
            "npm:@scope/fixture"
        } else {
            "git:github.com/acme/repo"
        };
        let query = Query::new(source.into(), Scope::Project, "/fixture/installed".into()).unwrap();
        let updates = run(package_queries::check_all(&fixture, &[query])).unwrap();
        assert_eq!(json!(!updates.is_empty()), row["value"], "{}", row["input"]);
        assert_eq!(
            json!(*fixture.calls.lock().unwrap()),
            row["calls"],
            "{}",
            row["input"]
        );
    }
}
struct Concurrent {
    live: AtomicUsize,
    maximum: AtomicUsize,
    completed: Mutex<Vec<String>>,
    fail: bool,
}
struct Lease<'a>(&'a AtomicUsize);
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Port for Concurrent {
    fn checkpoint(&self) -> Result<(), DomainError> {
        Ok(())
    }
    fn exists<'a>(&'a self, _: &'a str) -> PortFuture<'a, bool> {
        Box::pin(async { Ok(true) })
    }
    fn manifest<'a>(&'a self, _: &'a str) -> PortFuture<'a, Option<Value>> {
        Box::pin(async { Ok(Some(json!({"version":"1"}))) })
    }
    fn latest_npm<'a>(&'a self, name: &'a str, _: u64) -> PortFuture<'a, Option<Value>> {
        Box::pin(async move {
            let live = self.live.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(live, Ordering::SeqCst);
            let _lease = Lease(&self.live);
            let index: usize = name.strip_prefix('p').unwrap().parse().unwrap();
            let mut remaining = 8 - index % 8;
            std::future::poll_fn(|cx| {
                if remaining == 0 {
                    return Poll::Ready(());
                }
                remaining -= 1;
                cx.waker().wake_by_ref();
                Poll::Pending
            })
            .await;
            if self.fail && index == 3 {
                return Err(DomainError::Cancelled);
            }
            self.completed.lock().unwrap().push(name.into());
            Ok(Some(json!("2")))
        })
    }
    fn git<'a>(&'a self, _: &'a str, _: Vec<String>, _: bool, _: u64) -> PortFuture<'a, String> {
        panic!("unexpected Git")
    }
}
#[test]
fn four_concurrent_queries_preserve_order_and_drop_inflight_work_on_cancel() {
    let queries = (0..12)
        .map(|i| {
            Query::new(
                format!("npm:p{i}"),
                Scope::Project,
                format!("/fixture/p{i}"),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    for fail in [false, true] {
        let port = Concurrent {
            live: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            completed: Mutex::new(Vec::new()),
            fail,
        };
        let result = run(package_queries::check_all(&port, &queries));
        if fail {
            assert_eq!(result, Err(DomainError::Cancelled));
        } else {
            assert_eq!(
                result
                    .unwrap()
                    .iter()
                    .map(|u| u.source.clone())
                    .collect::<Vec<_>>(),
                (0..12).map(|i| format!("npm:p{i}")).collect::<Vec<_>>()
            );
            assert_ne!(port.completed.lock().unwrap()[0], "p0");
        }
        assert_eq!(port.maximum.load(Ordering::SeqCst), 4);
        assert_eq!(port.live.load(Ordering::SeqCst), 0);
    }
}
