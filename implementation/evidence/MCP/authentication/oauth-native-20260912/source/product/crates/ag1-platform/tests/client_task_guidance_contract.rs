#![cfg(target_os = "linux")]
use ag1_platform::{
    client::{Config, Host},
    cutover::Registry,
};
use ag1_protocol as w;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

async fn command(host: &Arc<Host>, id: &str, command: w::Command) {
    let s = host.commands.snapshot().unwrap();
    let reply = host
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: id.to_owned().try_into().unwrap(),
            scope: s.scope,
            expected_revision: s.revision,
            command,
            extension: None,
        })
        .await
        .unwrap();
    let w::BridgeFrame::Reply { status, data, .. } = reply else {
        panic!("Missing reply")
    };
    assert_eq!(status, "accepted");
    let operation = match serde_json::from_value::<w::CommandResult>(data).unwrap() {
        w::CommandResult::Accepted { operation } => operation,
        w::CommandResult::Settled { operation } => {
            assert_eq!(operation.status, w::OperationStatus::Completed);
            return;
        }
        other => panic!("{other:?}"),
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let view = host.settled_view().await.unwrap();
            assert!(view.error.is_none(), "{:?}", view.error);
            if let Some(terminal) = host.commands.snapshot().unwrap().last_terminal
                && terminal.operation == operation
            {
                assert_eq!(terminal.status, w::OperationStatus::Completed);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn task_state_guidance_tracks_actual_tool_availability_across_reopen() {
    prompt_roundtrip("Transport fixture", false).await;
}

#[tokio::test]
async fn native_default_prompt_rebuilds_after_tool_changes_and_reopen() {
    prompt_roundtrip("", false).await;
}

#[tokio::test]
async fn replacing_captured_default_rebuilds_an_existing_chat_without_losing_history() {
    prompt_roundtrip("Previously captured workspace prompt.", true).await;
}

async fn prompt_roundtrip(base_prompt: &str, migrate: bool) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    let authority = root.path().join("authority");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&authority).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o700)).unwrap();
    let registry = Registry::at(&authority).unwrap();
    registry.initialize_isolated(&workspace).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let wire = Arc::new(Mutex::new(vec![]));
    let captured = wire.clone();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        for _ in 0..4 {
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(v) => break v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
            let mut length = None;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
            let length = length.unwrap();
            assert!(length < 1024 * 1024);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            captured
                .lock()
                .unwrap()
                .push(serde_json::from_slice::<Value>(&body).unwrap());
            let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Fixture complete.\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3,\"total_tokens\":13}}\n\ndata: [DONE]\n\n";
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            socket.flush().unwrap();
        }
    });
    let config:Config=serde_json::from_value(json!({"models":[{"id":"Qwen-fixture","provider":"llamacpp","api":"openai-completions","baseUrl":format!("http://{address}/v1"),"compat":{"thinkingFormat":"qwen-chat-template"},"contextWindow":8192,"maxTokens":1024,"reasoning":true,"input":["text"]}],"selected":"Qwen-fixture","credential_env":null,"system_prompt":base_prompt,"prompt_append":"Host conversation boundary.","fixture":true,"policy_mode":"disabled"})).unwrap();
    let observed = Arc::new(Mutex::new(vec![]));
    let copy = observed.clone();
    let observer: ag1_platform::provider::http::RequestObserver = Arc::new(move |_, bytes| {
        copy.lock()
            .unwrap()
            .push(serde_json::from_slice::<Value>(bytes).unwrap());
        Ok(())
    });
    std::fs::create_dir(workspace.join(".ag1")).unwrap();
    std::fs::write(
        workspace.join(".ag1/APPEND_SYSTEM.md"),
        "Keep the user's appended instructions.",
    )
    .unwrap();
    std::fs::create_dir(workspace.join("src")).unwrap();
    std::fs::write(workspace.join("before-open.txt"), "before").unwrap();
    let mut host = Host::open_with_request_observer(
        &workspace,
        &state,
        config.clone(),
        &registry,
        observer.clone(),
    )
    .await
    .unwrap();
    std::fs::write(workspace.join("after-open.txt"), "after").unwrap();
    for (index, enabled) in [false, false, true, false].iter().enumerate() {
        if index == 1 {
            host.shutdown().await.unwrap();
            drop(host);
            let mut reopened_config = config.clone();
            if migrate {
                reopened_config.system_prompt.clear();
            }
            host = Host::open_with_request_observer(
                &workspace,
                &state,
                reopened_config,
                &registry,
                observer.clone(),
            )
            .await
            .unwrap();
        }
        if index == 1 {
            std::fs::write(workspace.join("after-reopen.txt"), "later").unwrap();
        }
        command(
            &host,
            &format!("tools-{index}"),
            w::Command::Runtime {
                action: "tools".into(),
                args: json!({"names": if *enabled { vec!["taskState"] } else { vec![] }}),
            },
        )
        .await;
        command(
            &host,
            &format!("prompt-{index}"),
            w::Command::Prompt {
                input: w::PromptInput::Inline {
                    text: "Return the fixture marker.".into(),
                },
            },
        )
        .await;
    }
    host.shutdown().await.unwrap();
    server.join().unwrap();
    let wire = wire.lock().unwrap();
    assert_eq!(*wire, *observed.lock().unwrap());
    for (index, (request, enabled)) in wire.iter().zip([false, false, true, false]).enumerate() {
        let system = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "system" || message["role"] == "developer")
            .unwrap()["content"]
            .as_str()
            .unwrap();
        assert!(system.contains("Top-level workspace entries (snapshot at session start):"));
        assert!(system.contains("src/\n"));
        assert!(system.contains("Keep the user's appended instructions."), "{system}");
        assert!(system.contains("Host conversation boundary."));
        if base_prompt.is_empty() || migrate && index > 0 {
            assert!(!system.contains("Previously captured workspace prompt."));
            assert!(system.contains("Host-supplied model identity:"));
            assert!(system.contains("Qwen-fixture"));
            assert_eq!(system.contains("- taskState:"), enabled);
            assert!(!system.contains("- check:") && !system.contains("- bash:"));
            assert!(!system.contains("Cruzible") && !system.contains("2026-09-07"));
        }

        assert!(system.contains("before-open.txt"));
        // A reopen captures the current directory; tool changes within that
        // incarnation retain its original observation.
        assert_eq!(system.contains("after-open.txt"), index > 0);
        assert!(!system.contains("after-reopen.txt"));
        let advertised = request["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|tool| tool["function"]["name"] == "taskState");
        assert_eq!(advertised, enabled);
        let user = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|message| message["role"] == "user")
            .unwrap();
        let content = user["content"].to_string();
        assert!(content.contains("latest_user_instruction"));
        assert!(content.contains("Return the fixture marker."));
        if enabled {
            assert!(content.contains("update with taskState"));
            assert!(!content.contains("read-only in this turn"));
        } else {
            assert!(
                !content.contains("update with taskState"),
                "Unavailable tool requested: {content}"
            );
            assert!(!content.contains("choose and record"));
            assert!(content.contains("read-only in this turn"));
            assert!(content.contains("Shell output cannot update"));
        }
    }
}
