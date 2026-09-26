#![cfg(target_os = "linux")]
use ag1_platform::{
    client::{Config, Host},
    cutover::Registry,
};
use ag1_protocol as w;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

static NEXT_COMMAND: AtomicUsize = AtomicUsize::new(1);
async fn dispatch(host: &Arc<Host>, command: w::Command) -> Value {
    let s = host.commands.snapshot().unwrap();
    let result = host
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: format!("test-{}", NEXT_COMMAND.fetch_add(1, Ordering::SeqCst))
                .try_into()
                .unwrap(),
            scope: s.scope,
            expected_revision: s.revision,
            command,
            extension: None,
        })
        .await
        .unwrap();
    let w::BridgeFrame::Reply { status, data, .. } = result else {
        panic!("no reply")
    };
    assert_ne!(status, "rejected", "{data}");
    data
}
fn endpoint() -> (
    String,
    Arc<AtomicUsize>,
    Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let n = seen.clone();
    let go = release.clone();
    let handle = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let (mut socket, _) = loop {
            if let Ok(s) = listener.accept() {
                break s;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    length = v.trim().parse().unwrap();
                }
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let request: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["role"] == "user")
        );
        n.fetch_add(1, Ordering::SeqCst);
        while !go.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Independent answer\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let _ = write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    });
    (url, seen, release, handle)
}
// This fixture owns the server threads. Cancellation acknowledges only after the
// addressed endpoint has actually exited, matching local scheduler custody.
struct OwnedFixture {
    endpoints: Vec<(
        String,
        Arc<std::sync::atomic::AtomicBool>,
        std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    )>,
}
impl ag1_platform::provider::http::GenerationRuntime for OwnedFixture {
    fn prepare<'a>(
        &'a self,
        _: &'a ag1_platform::provider::http::HttpRequest,
    ) -> ag1_core::PortFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn cancel_generation<'a>(&'a self, url: &'a str) -> ag1_core::PortFuture<'a, ()> {
        Box::pin(async move {
            let (_, go, handle) = self
                .endpoints
                .iter()
                .find(|(base, _, _)| url.starts_with(base))
                .unwrap();
            go.store(true, Ordering::SeqCst);
            let handle = handle.lock().unwrap().take();
            if let Some(handle) = handle {
                tokio::task::spawn_blocking(move || handle.join().unwrap())
                    .await
                    .unwrap();
            }
            Ok(())
        })
    }
    fn shutdown(&self) -> ag1_core::PortFuture<'_, ()> {
        Box::pin(async move {
            for (url, _, _) in &self.endpoints {
                self.cancel_generation(url).await?;
            }
            Ok(())
        })
    }
}
async fn wait(label: &str, mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Timed out: {label}"));
}
#[tokio::test]
async fn conversations_run_independently_preserve_history_and_scope_approvals() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let state = temp.path().join("state");
    let authority = temp.path().join("authority");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&authority).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o700)).unwrap();
    let registry = Registry::at(&authority).unwrap();
    registry.initialize_isolated(&workspace).unwrap();
    let (url_a, seen_a, release_a, server_a) = endpoint();
    let (url_b, seen_b, release_b, server_b) = endpoint();
    let config:Config=serde_json::from_value(json!({"models":[{"provider":"fixture","id":"a","api":"openai-completions","baseUrl":url_a,"contextWindow":16000,"maxTokens":512},{"provider":"fixture","id":"b","api":"openai-completions","baseUrl":url_b,"contextWindow":16000,"maxTokens":512}],"selected":"a","credential_env":null,"system_prompt":"Test","fixture":true})).unwrap();
    let runtime = Arc::new(OwnedFixture {
        endpoints: vec![
            (
                url_a.clone(),
                release_a.clone(),
                std::sync::Mutex::new(Some(server_a)),
            ),
            (
                url_b.clone(),
                release_b.clone(),
                std::sync::Mutex::new(Some(server_b)),
            ),
        ],
    });
    let root = Host::open_rehearsal_with_runtime(&workspace, &state, config, &registry, runtime)
        .await
        .unwrap();
    let a = root.open_conversation(None).await.unwrap();
    let a_id = a
        .commands
        .snapshot()
        .unwrap()
        .scope
        .session
        .as_str()
        .to_owned();
    dispatch(
        &a,
        w::Command::Runtime {
            action: "approval_policy".into(),
            args: json!({"mode":"disabled"}),
        },
    )
    .await;
    dispatch(
        &a,
        w::Command::Prompt {
            input: w::PromptInput::Inline {
                text: "First chat".into(),
            },
        },
    )
    .await;
    wait("A provider", || {
        let v = a.view().unwrap();
        assert!(v.error.is_none(), "A error: {:?}", v.error);
        if a.commands.snapshot().unwrap().queued.is_empty() {
            panic!("A finished without request: {v:?}");
        }
        seen_a.load(Ordering::SeqCst) == 1
    })
    .await;
    let b = root.open_conversation(None).await.unwrap();
    let b_id = b
        .commands
        .snapshot()
        .unwrap()
        .scope
        .session
        .as_str()
        .to_owned();
    assert!(Arc::ptr_eq(
        &a,
        &root.open_conversation(Some(&a_id)).await.unwrap()
    ));
    assert_eq!(
        b.view().unwrap().details.context["approval_policy"]["mode"],
        "interactive"
    );
    dispatch(
        &b,
        w::Command::Model {
            action: "select".into(),
            args: json!({"provider":"fixture","model_id":"b"}),
        },
    )
    .await;
    wait("B model", || {
        b.commands.snapshot().unwrap().queued.is_empty()
    })
    .await;
    dispatch(
        &b,
        w::Command::Prompt {
            input: w::PromptInput::Inline {
                text: "Second chat".into(),
            },
        },
    )
    .await;
    wait("B provider", || {
        let v = b.view().unwrap();
        assert!(v.error.is_none(), "B error: {:?}", v.error);
        seen_b.load(Ordering::SeqCst) == 1
    })
    .await;
    assert!(!a.commands.snapshot().unwrap().queued.is_empty());
    assert!(!b.commands.snapshot().unwrap().queued.is_empty());
    assert!(
        root.idle_client_guard().await.is_err(),
        "An idle chat cannot change a repository while sibling chats run"
    );
    let root_snapshot = root.commands.snapshot().unwrap();
    assert!(
        root.dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: "duplicate-resume".to_string().try_into().unwrap(),
            scope: root_snapshot.scope,
            expected_revision: root_snapshot.revision,
            command: w::Command::Session {
                action: "resume".into(),
                args: json!({"session":a_id})
            },
            extension: None
        })
        .await
        .is_err()
    );
    let a_op = a.commands.snapshot().unwrap().queued[0].clone();
    let running=a.commands.snapshot().unwrap();
    assert!(a.dispatch(w::CommandEnvelope {version:Default::default(),id:"running-folder-change".to_owned().try_into().unwrap(),scope:running.scope,expected_revision:running.revision,command:w::Command::Runtime {action:"folder_access".into(),args:json!({"revision":1,"action":"mode","path":workspace,"mode":"read_only"})},extension:None}).await.is_err());
    dispatch(&a, w::Command::Cancel { operation: a_op }).await;
    release_a.store(true, Ordering::SeqCst);
    wait("A cancellation", || {
        assert!(
            a.view().unwrap().error.is_none(),
            "cancel error: {:?}",
            a.view().unwrap().error
        );
        a.commands.snapshot().unwrap().queued.is_empty()
    })
    .await;
    assert!(
        !b.commands.snapshot().unwrap().queued.is_empty(),
        "Stopping A must not stop B"
    );
    release_b.store(true, Ordering::SeqCst);
    wait("B completion", || {
        b.commands.snapshot().unwrap().queued.is_empty()
    })
    .await;
    assert_eq!(
        b.commands.snapshot().unwrap().last_terminal.unwrap().status,
        w::OperationStatus::Completed
    );
    assert_eq!(
        a.view()
            .unwrap()
            .details
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .count(),
        1
    );
    assert_eq!(
        b.view()
            .unwrap()
            .details
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .count(),
        1
    );
    drop(
        root.idle_client_guard()
            .await
            .expect("Repository controls reopen after all project conversations settle"),
    );
    let folder=temp.path().join("second-folder"); std::fs::create_dir(&folder).unwrap();
    let history=serde_json::to_value(&b.view().unwrap().details.messages).unwrap();
    let added=dispatch(&b,w::Command::Runtime {action:"folder_access".into(),args:json!({"revision":1,"action":"add","path":folder,"mode":"read_only"})}).await;
    dispatch(&b,w::Command::Runtime {action:"folder_access".into(),args:json!({"revision":added["revision"],"action":"primary","path":folder})}).await;
    assert_eq!(serde_json::to_value(&b.view().unwrap().details.messages).unwrap(),history);
    let a_scope = a.commands.snapshot().unwrap().scope;
    let wrong = b
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: "wrong-chat".to_string().try_into().unwrap(),
            scope: a_scope,
            expected_revision: b.commands.snapshot().unwrap().revision,
            command: w::Command::Runtime {
                action: "approval_policy".into(),
                args: json!({"mode":"disabled"}),
            },
            extension: None,
        })
        .await;
    assert!(wrong.is_err());
    b.shutdown().await.unwrap();
    a.shutdown().await.unwrap();
    let reopened = root.open_conversation(Some(&b_id)).await.unwrap();
    assert!(reopened.view().unwrap().details.messages.iter().any(|m| {
        m.blocks
            .iter()
            .any(|b| matches!(b,w::MessageBlock::Text{text} if text=="Independent answer"))
    }));
    reopened.shutdown().await.unwrap();
    root.shutdown().await.unwrap();
}
