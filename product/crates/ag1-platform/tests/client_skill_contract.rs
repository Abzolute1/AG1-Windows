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
async fn skills_reach_native_provider_requests_for_auto_and_explicit_use() {
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
    let config:Config=serde_json::from_value(json!({"models":[{"id":"Qwen-fixture","provider":"llamacpp","api":"openai-completions","baseUrl":format!("http://{address}/v1"),"compat":{"thinkingFormat":"qwen-chat-template"},"contextWindow":32768,"maxTokens":1024,"reasoning":true,"input":["text","image"]}],"selected":"Qwen-fixture","credential_env":null,"system_prompt":"","prompt_append":"Host conversation boundary.","fixture":true,"policy_mode":"disabled"})).unwrap();
    let dir = workspace.join(".ag1/skills/svg-art");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), "---\nname: svg-art\ndescription: SVG artwork\ntriggers: [pelican]\n---\nTARGETED_EDITS_MARKER: preserve geometry and preview.").unwrap();
    let host = Host::open_with_registry(&workspace, &state, config, &registry)
        .await
        .unwrap();
    for (index, text) in [
        "Draw a pelican",
        "/skill:svg-art Draw a bird",
        "Hello unrelated",
    ]
    .iter()
    .enumerate()
    {
        let cmd = if index == 1 {
            w::Command::Runtime {
                action: "client_action".into(),
                args: json!({"text":text}),
            }
        } else {
            w::Command::Prompt {
                input: w::PromptInput::Inline {
                    text: (*text).into(),
                },
            }
        };
        command(&host, &format!("skill-{index}"), cmd).await;
    }
    // Same upload/message-artifact route used by the desktop attachment composer.
    use base64::Engine;
    async fn upload_call(host: &Arc<Host>, id: &str, action: &str, args: Value) -> Value {
        let snapshot = host.commands.snapshot().unwrap();
        let reply = host
            .dispatch(w::CommandEnvelope {
                version: Default::default(),
                id: id.to_owned().try_into().unwrap(),
                scope: snapshot.scope,
                expected_revision: snapshot.revision,
                command: w::Command::Artifacts {
                    action: action.into(),
                    args,
                },
                extension: None,
            })
            .await
            .unwrap();
        let w::BridgeFrame::Reply { data, .. } = reply else {
            panic!("missing upload reply")
        };
        data
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let message = json!({"role":"user","content":[{"type":"text","text":"Draw a pelican from this image"},{"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(png.into_inner())}]});
    let bytes = serde_json::to_vec(&message).unwrap();
    let upload = upload_call(&host, "upload-start", "upload_begin", json!({})).await;
    upload_call(&host,"upload-data","upload_append",json!({"upload":upload["upload"],"offset":"0","base64":base64::engine::general_purpose::STANDARD.encode(&bytes)})).await;
    let result = upload_call(
        &host,
        "upload-end",
        "upload_finish",
        json!({"upload":upload["upload"],"sha256":ag1_storage::digest(&bytes)}),
    )
    .await;
    command(
        &host,
        "skill-image",
        w::Command::Prompt {
            input: w::PromptInput::MessageArtifact {
                artifact: result["artifact"]
                    .as_str()
                    .unwrap()
                    .to_owned()
                    .try_into()
                    .unwrap(),
            },
        },
    )
    .await;
    host.shutdown().await.unwrap();
    server.join().unwrap();
    for (index, request) in wire.lock().unwrap().iter().enumerate() {
        let last = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|m| {
                m["role"] == "user"
                    && !m["content"]
                        .to_string()
                        .contains("AG1 REQUEST-LOCAL WORKING STATE")
            })
            .unwrap()["content"]
            .to_string();
        assert_eq!(last.contains("TARGETED_EDITS_MARKER"), index != 2, "{last}");
        assert_eq!(
            last.matches("TARGETED_EDITS_MARKER").count(),
            usize::from(index != 2)
        );
        if index == 3 {
            assert!(
                last.contains("data:image/png;base64,"),
                "image lost: {last}"
            );
        }
    }
}
