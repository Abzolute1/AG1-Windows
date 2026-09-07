use ag1_platform::client::{Config, Host, actions};
use ag1_protocol as w;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    time::Duration,
};

#[test]
fn measured_cutover_commands_keep_results_private_and_never_retry_an_existing_receipt() {
    use ag1_platform::cutover::{Registry, Stage};
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let state = temp.path().join("state");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&state).unwrap();
    let registry = Registry::for_user().unwrap();
    let mut owner = registry.open_typescript(&workspace, &state).unwrap();
    owner.quiesced().unwrap();
    drop(owner);
    let receipt = temp.path().join("measurement");
    let output = Command::new(env!("CARGO_BIN_EXE_ag1-cutover"))
        .arg("measure")
        .arg(&receipt)
        .arg("begin-transfer")
        .arg(&workspace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        registry.inspect(&workspace).unwrap().unwrap().stage,
        Stage::Draining
    );
    let start = fs::read(receipt.join("started.json")).unwrap();
    let result = fs::read(receipt.join("result.json")).unwrap();
    let record: Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(record["started_sha256"], ag1_storage::digest(&start));
    assert_eq!(record["status"], "command_succeeded");
    assert_eq!(
        record["result"],
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    );
    assert!(record["elapsed_ms"].is_u64());
    assert_eq!(record["phase_acceptance"], false);
    assert_eq!(
        fs::metadata(&receipt).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for name in ["started.json", "result.json"] {
        assert_eq!(
            fs::metadata(receipt.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let duplicate = Command::new(env!("CARGO_BIN_EXE_ag1-cutover"))
        .arg("measure")
        .arg(&receipt)
        .arg("cancel-unaccepted")
        .arg(&workspace)
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("no command executed"));
    assert_eq!(fs::read(receipt.join("started.json")).unwrap(), start);
    assert_eq!(fs::read(receipt.join("result.json")).unwrap(), result);
    assert_eq!(
        registry.inspect(&workspace).unwrap().unwrap().stage,
        Stage::Draining
    );
    registry
        .resume_transfer(&workspace)
        .unwrap()
        .cancel_transfer()
        .unwrap();
    let failed = temp.path().join("failed");
    let output = Command::new(env!("CARGO_BIN_EXE_ag1-cutover"))
        .arg("measure")
        .arg(&failed)
        .arg("unsupported-action")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let record: Value =
        serde_json::from_slice(&fs::read(failed.join("result.json")).unwrap()).unwrap();
    assert_eq!(record["status"], "command_failed");
    assert_eq!(record["automatic_retry_allowed"], false);
}

#[test]
fn interrupted_measurement_retains_intent_without_claiming_a_command_result() {
    use ag1_platform::cutover::measurement::Measurement;
    let temp = tempfile::tempdir().unwrap();
    let receipt = temp.path().join("interrupted");
    let measurement =
        Measurement::begin(&receipt, &["import".into(), "fixture-workspace".into()]).unwrap();
    drop(measurement); // Crash boundary before durable result publication.
    assert!(receipt.join("started.json").exists());
    assert!(!receipt.join("result.json").exists());
    assert!(Measurement::begin(&receipt, &["import".into()]).is_err());
    let receipt = temp.path().join("publication-conflict");
    let measurement =
        Measurement::begin(&receipt, &["inspect".into(), "fixture-workspace".into()]).unwrap();
    std::fs::write(receipt.join("result.json"), "retained conflicting evidence").unwrap();
    assert!(measurement.finish(&Ok(json!({"observed":true}))).is_err());
    assert_eq!(
        std::fs::read_to_string(receipt.join("result.json")).unwrap(),
        "retained conflicting evidence"
    );
}
fn provider() -> (String, std::thread::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut input = vec![];
        let mut buf = [0; 4096];
        let body = loop {
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0);
            input.extend_from_slice(&buf[..n]);
            if let Some(end) = input.windows(4).position(|s| s == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&input[..end]);
                let length: usize = header
                    .lines()
                    .find_map(|s| {
                        s.to_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .unwrap()
                    .parse()
                    .unwrap();
                if input.len() >= end + 4 + length {
                    break serde_json::from_slice::<Value>(&input[end + 4..end + 4 + length])
                        .unwrap();
                }
            }
        };
        std::thread::sleep(Duration::from_millis(40));
        let body_out = format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":"Hello from native"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":3}})
        );
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body_out.len(),body_out).unwrap();
        body
    });
    (origin, thread)
}
fn config(origin: &str) -> Config {
    // Tests that never generate still own distinct loopback endpoints. Sharing
    // the old discard port would violate P13's cross-host endpoint lease.
    thread_local! {
        static UNUSED_PROVIDER: TcpListener = TcpListener::bind("127.0.0.1:0").unwrap();
    }
    let origin = if origin == "http://127.0.0.1:9" {
        UNUSED_PROVIDER.with(|socket| format!("http://{}", socket.local_addr().unwrap()))
    } else {
        origin.to_owned()
    };
    Config {
        policy_mode: "interactive".into(),
        max_generations: None,
        tooling: None,
        extensions: None,
        packages: None,
        models: vec![
            json!({"provider":"fixture","id":"initial","api":"openai-completions","baseUrl":origin,"contextWindow":32768,"maxTokens":4096,"input":["text"],"reasoning":false}),
        ],
        selected: "initial".into(),
        credential_env: None,
        imported: None,
        runtime: None,
        credentials: Default::default(),
        system_prompt: "Isolated fixture".into(),
        fixture: true,
    }
}
async fn host(origin: &str) -> (tempfile::TempDir, Arc<Host>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    Host::init(&root).unwrap();
    let host = Host::open(&root, &dir.path().join("private"), config(origin))
        .await
        .unwrap();
    (dir, host)
}
async fn wait(host: &Host) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let view = host.settled_view().await.unwrap();
            assert!(view.error.is_none(), "{:?}", view.error);
            if view.snapshot.status == "idle" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn detached_projection_does_not_cancel_and_jsonl_matches_desktop_domain() {
    let (origin, server) = provider();
    let (_dir, host) = host(&origin).await;
    let envelope = ag1_cli::envelope(&host, "Say hello").unwrap();
    let id = envelope.id.clone();
    let frame = host.dispatch(envelope.clone()).await.unwrap();
    assert!(matches!(frame,w::BridgeFrame::Reply{status,..} if status=="accepted"));
    // Dropping an actual native subscription only detaches the projection.
    let sub = host.commands.subscribe().unwrap();
    host.commands.unsubscribe(&sub.id).unwrap();
    wait(&host).await;
    let view = host.view().unwrap();
    assert!(view.details.messages.iter().any(|m| {
        m.blocks
            .iter()
            .any(|b| matches!(b,w::MessageBlock::Text{text} if text=="Hello from native"))
    }));
    let frame = w::WireMessage::Client(Box::new(view));
    let bytes = w::encode(&frame).unwrap();
    assert_eq!(w::decode(&bytes).unwrap(), frame);
    let duplicate = host.dispatch(envelope).await.unwrap();
    assert!(matches!(duplicate,w::BridgeFrame::Reply{id:reply,..} if reply==id));
    assert_eq!(
        server.join().unwrap()["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"][0]["text"],
        "Say hello"
    );
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn print_uses_native_completion_and_clean_binary_stdout() {
    let (origin, server) = provider();
    let (_dir, host) = host(&origin).await;
    let (text, code) = ag1_cli::print(&host, "Say hello").await.unwrap();
    assert_eq!(text, "Hello from native");
    assert_eq!(code, 0);
    server.join().unwrap();
    host.shutdown().await.unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ag1-cli"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "ag1-cli 0.1.0\n");
    assert!(output.stderr.is_empty());
}
#[tokio::test]
async fn file_cas_failure_retains_disk_and_stale_scope_denied() {
    let (_dir, host) = host("http://127.0.0.1:9").await;
    std::fs::write(host.workspace.root().join("a.txt"), "before").unwrap();
    let mut command = ag1_cli::envelope(&host, "unused").unwrap();
    command.command = w::Command::Files {
        action: "read".into(),
        args: json!({"path":"a.txt"}),
    };
    let w::BridgeFrame::Reply { data, .. } = host.dispatch(command.clone()).await.unwrap() else {
        panic!()
    };
    std::fs::write(host.workspace.root().join("a.txt"), "external").unwrap();
    command.command = w::Command::Files {
        action: "save".into(),
        args: json!({"path":"a.txt","text":"dirty buffer","revision":data["revision"]}),
    };
    assert!(
        matches!(host.dispatch(command.clone()).await,Ok(w::BridgeFrame::Reply {status,..}) if status=="rejected")
    );
    assert_eq!(
        std::fs::read_to_string(host.workspace.root().join("a.txt")).unwrap(),
        "external"
    );
    command.scope.epoch = w::Sequence::new(99);
    assert!(host.dispatch(command).await.is_err());
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn second_writer_and_unmarked_workspace_rejected() {
    let (dir, host) = host("http://127.0.0.1:9").await;
    assert!(
        Host::open(
            host.workspace.root(),
            &dir.path().join("private"),
            config("http://127.0.0.1:9")
        )
        .await
        .is_err()
    );
    let ordinary = dir.path().join("ordinary");
    std::fs::create_dir(&ordinary).unwrap();
    assert!(
        Host::open(
            &ordinary,
            &dir.path().join("other"),
            config("http://127.0.0.1:9")
        )
        .await
        .is_err()
    );
    host.shutdown().await.unwrap();
}
#[test]
fn slash_commands_do_not_invent_dag_or_dead_hooks() {
    assert!(matches!(
        actions::parse("/workflow workflow.json").unwrap(),
        w::Command::Runtime { .. }
    ));
    assert!(actions::parse("/hooks").is_err());
    assert!(actions::parse("/model missing-provider").is_err());
    assert!(
        matches!(actions::parse("/compact preserve task").unwrap(),w::Command::Session{action,..} if action=="compact")
    );
}
#[tokio::test]
async fn export_escapes_untrusted_blocks_and_never_embeds_invoke() {
    let (_dir, host) = host("http://127.0.0.1:9").await;
    let mut view = host.view().unwrap();
    view.details.messages.push(w::ClientMessage {
        id: "x".into(),
        role: "assistant".into(),
        blocks: vec![w::MessageBlock::Text {
            text: "<script>__TAURI_INTERNALS__.invoke('quit')</script>".into(),
        }],
    });
    let html = ag1_platform::client::export::html(&view).unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("default-src 'none'"));
    assert!(html.contains("Mario Zechner"));
    host.shutdown().await.unwrap();
}

async fn command(host: &Arc<Host>, command: w::Command) -> w::BridgeFrame {
    let mut envelope = ag1_cli::envelope(host, "unused").unwrap();
    envelope.command = command;
    host.dispatch(envelope).await.unwrap()
}
fn data(frame: w::BridgeFrame) -> Value {
    match frame {
        w::BridgeFrame::Reply { status, data, .. } => {
            assert_ne!(status, "rejected", "{data}");
            data
        }
        _ => panic!("expected reply"),
    }
}
#[tokio::test]
async fn context_selection_persists_and_model_conflict_is_read_only() {
    let (dir, host) = host("http://127.0.0.1:9").await;
    data(
        command(
            &host,
            w::Command::Model {
                action: "context".into(),
                args: json!({"tokens":8192}),
            },
        )
        .await,
    );
    assert_eq!(
        host.view().unwrap().details.context["working_context_limit"],
        8192
    );
    let root = host.workspace.root().to_owned();
    host.shutdown().await.unwrap();
    drop(host);
    let mut cfg = config("http://127.0.0.1:9");
    let mut conflict = cfg.models[0].clone();
    conflict["id"] = json!("conflicting");
    conflict["runtimeConflict"] = json!("Pinned binary mismatch");
    cfg.models.push(conflict);
    let reopened = Host::open(&root, &dir.path().join("private"), cfg)
        .await
        .unwrap();
    assert_eq!(
        reopened.view().unwrap().details.context["working_context_limit"],
        8192
    );
    let before = std::fs::read_dir(&root).unwrap().count();
    assert!(
        matches!(command(&reopened,w::Command::Model {action:"select".into(),args:json!({"provider":"fixture","model_id":"conflicting"})}).await,w::BridgeFrame::Reply {status,..} if status=="rejected")
    );
    assert_eq!(reopened.view().unwrap().snapshot.model, "initial");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), before);
    reopened.shutdown().await.unwrap();
}
#[tokio::test]
async fn message_upload_validates_hash_preserves_document_and_can_repeat_bytes() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let (_dir, host) = host("http://127.0.0.1:9").await;
    let bytes = serde_json::to_vec(
        &json!({"role":"user","content":[{"type":"text","text":"attached context"}]}),
    )
    .unwrap();
    let mut ids = vec![];
    for _ in 0..2 {
        let started = data(
            command(
                &host,
                w::Command::Artifacts {
                    action: "upload_begin".into(),
                    args: json!({}),
                },
            )
            .await,
        );
        let id = &started["upload"];
        data(
            command(
                &host,
                w::Command::Artifacts {
                    action: "upload_append".into(),
                    args: json!({"upload":id,"offset":"0","base64":STANDARD.encode(&bytes)}),
                },
            )
            .await,
        );
        let mut invalid = ag1_cli::envelope(&host, "unused").unwrap();
        invalid.command = w::Command::Artifacts {
            action: "upload_finish".into(),
            args: json!({"upload":id,"sha256":"wrong"}),
        };
        assert!(
            matches!(host.dispatch(invalid).await,Ok(w::BridgeFrame::Reply {status,..}) if status=="rejected")
        );
        let finished = data(
            command(
                &host,
                w::Command::Artifacts {
                    action: "upload_finish".into(),
                    args: json!({"upload":id,"sha256":ag1_storage::digest(&bytes)}),
                },
            )
            .await,
        );
        let artifact = finished["artifact"].as_str().unwrap().to_owned();
        let page = data(
            command(
                &host,
                w::Command::ArtifactRead {
                    artifact: artifact.clone().try_into().unwrap(),
                    offset: w::Sequence::new(0),
                    limit: 32768,
                },
            )
            .await,
        );
        assert_eq!(
            STANDARD.decode(page["base64"].as_str().unwrap()).unwrap(),
            bytes
        );
        ids.push(artifact);
    }
    assert_ne!(ids[0], ids[1]);
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn extension_command_ui_is_native_queued_correlated_and_not_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    Host::init(&root).unwrap();
    std::fs::create_dir_all(root.join(".ag1/extensions")).unwrap();
    std::fs::write(root.join(".ag1/extensions/review.ts"),r#"export default function(api) { api.registerFlag('review-mode',{type:'boolean',default:false}); api.registerShortcut('ctrl+shift+j',{handler:async(ctx)=>ctx.ui.notify('Shortcut handled','info')}); api.registerShortcut('ctrl+s',{handler:()=>{throw Error('reserved must not run')}}); api.registerCommand('review', { description:'Review fixture', handler:async(args,ctx)=>{ const answer=await ctx.ui.select('Choose result',['keep','revise']); ctx.ui.notify('Selected '+answer,'info'); } }); }"#).unwrap();
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let mut cfg = config("http://127.0.0.1:9");
    cfg.extensions = Some(ag1_platform::client::extensions::Extensions {
        node: repository.join(".local/toolchains/node-v24.14.0-linux-x64/bin/node"),
        host_script: repository.join("product/compat/extensions/host.mjs"),
        process_host: repository.join("product/target/debug/ag1-process-host"),
        trusted: true,
        allow_inference: false,
    });
    let host = Host::open(&root, &dir.path().join("private"), cfg)
        .await
        .unwrap();
    let catalog = data(
        command(
            &host,
            w::Command::Runtime {
                action: "extension_catalog".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert!(
        catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "review")
    );
    let mut envelope = ag1_cli::envelope(&host, "unused").unwrap();
    envelope.command = w::Command::Runtime {
        action: "extension_command".into(),
        args: json!({"name":"review","arguments":""}),
    };
    let reply = host.dispatch(envelope.clone()).await.unwrap();
    assert!(matches!(reply, w::BridgeFrame::Reply { durable: true, .. }));
    let pending = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let ui = data(
                command(
                    &host,
                    w::Command::Runtime {
                        action: "extension_ui".into(),
                        args: json!({}),
                    },
                )
                .await,
            );
            if !ui["pending"].is_null() {
                break ui["pending"].clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pending["method"], "ui.select");
    let mut invalid = ag1_cli::envelope(&host, "unused").unwrap();
    invalid.command = w::Command::Runtime {
        action: "extension_answer".into(),
        args: json!({"id":pending["id"],"value":"forged-option"}),
    };
    assert!(
        matches!(host.dispatch(invalid).await,Ok(w::BridgeFrame::Reply {status,..}) if status=="rejected")
    );
    data(
        command(
            &host,
            w::Command::Runtime {
                action: "extension_answer".into(),
                args: json!({"id":pending["id"],"value":"keep"}),
            },
        )
        .await,
    );
    wait(&host).await;
    let ui = data(
        command(
            &host,
            w::Command::Runtime {
                action: "extension_ui".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert_eq!(ui["events"][0]["arguments"]["args"][0], "Selected keep");
    data(host.dispatch(envelope).await.unwrap());
    assert_eq!(
        data(
            command(
                &host,
                w::Command::Runtime {
                    action: "extension_ui".into(),
                    args: json!({})
                }
            )
            .await
        )["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let settings = data(
        command(
            &host,
            w::Command::Runtime {
                action: "extension_settings".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert_eq!(settings["flags"]["review-mode"], false);
    command(
        &host,
        w::Command::Runtime {
            action: "extension_flag".into(),
            args: json!({"name":"review-mode","value":true}),
        },
    )
    .await;
    wait(&host).await;
    let settings = data(
        command(
            &host,
            w::Command::Runtime {
                action: "extension_settings".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert_eq!(settings["flags"]["review-mode"], true);
    let mut reserved = ag1_cli::envelope(&host, "unused").unwrap();
    reserved.command = w::Command::Runtime {
        action: "extension_shortcut".into(),
        args: json!({"key":"ctrl+s","keybindings":{}}),
    };
    host.reply(reserved).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while host.view().unwrap().snapshot.status != "idle" {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        host.view().unwrap().details.activity["last_terminal"]["status"],
        "failed"
    );
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn skill_creation_requires_current_native_approval_and_retains_receipt() {
    let (_dir, host) = host("http://127.0.0.1:9").await;
    data(command(&host,w::Command::Runtime {action:"resource_action".into(),args:json!({"name":"createSkill","arguments":{"name":"review-code","description":"Review contained project changes"}})}).await);
    let binding = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let view = host.view().unwrap();
            if let Some(p) = view.snapshot.pending.first() {
                break p.binding.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !host
            .workspace
            .root()
            .join(".ag1/skills/review-code/SKILL.md")
            .exists()
    );
    let mut stale = binding.clone();
    stale.argument_sha256 = "invalid".into();
    assert!(
        matches!(command(&host,w::Command::Approve {binding:stale,allow:true}).await,w::BridgeFrame::Reply {status,..} if status=="rejected")
    );
    data(
        command(
            &host,
            w::Command::Approve {
                binding,
                allow: true,
            },
        )
        .await,
    );
    wait(&host).await;
    assert!(
        std::fs::read_to_string(
            host.workspace
                .root()
                .join(".ag1/skills/review-code/SKILL.md")
        )
        .unwrap()
        .contains("Review contained project changes")
    );
    assert_eq!(
        host.commands
            .snapshot()
            .unwrap()
            .last_terminal
            .unwrap()
            .status,
        w::OperationStatus::Completed
    );
    let resources = data(
        command(
            &host,
            w::Command::Runtime {
                action: "resources".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert!(
        resources["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "review-code")
    );
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn shutdown_cancels_resource_approval_without_creating_skill() {
    let (_dir, host) = host("http://127.0.0.1:9").await;
    data(command(&host,w::Command::Runtime {action:"resource_action".into(),args:json!({"name":"createSkill","arguments":{"name":"never-created","description":"cancel fixture"}})}).await);
    tokio::time::timeout(Duration::from_secs(10), async {
        while host.view().unwrap().snapshot.pending.is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    host.shutdown().await.unwrap();
    assert!(
        !host
            .workspace
            .root()
            .join(".ag1/skills/never-created/SKILL.md")
            .exists()
    );
    assert!(host.commands.snapshot().unwrap().queued.is_empty());
}
#[tokio::test]
async fn explicitly_granted_extension_inference_uses_native_transport_and_retains_output() {
    let (origin, server) = provider();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    Host::init(&root).unwrap();
    std::fs::create_dir_all(root.join(".ag1/extensions")).unwrap();
    std::fs::write(root.join(".ag1/extensions/infer.ts"),r#"import {complete} from '@ag1/ai'; export default api => { api.registerCommand('infer',{handler:async(_,ctx)=>{const result=await complete({provider:'fixture',id:'initial',baseUrl:'https://forged.invalid'},{messages:[{role:'user',content:'extension request',timestamp:1}]});ctx.ui.notify(result.content[0].text,'info');}}); };"#).unwrap();
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let mut cfg = config(&origin);
    cfg.extensions = Some(ag1_platform::client::extensions::Extensions {
        node: repository.join(".local/toolchains/node-v24.14.0-linux-x64/bin/node"),
        host_script: repository.join("product/compat/extensions/host.mjs"),
        process_host: repository.join("product/target/debug/ag1-process-host"),
        trusted: true,
        allow_inference: true,
    });
    let host = Host::open(&root, &dir.path().join("private"), cfg)
        .await
        .unwrap();
    data(
        command(
            &host,
            w::Command::Runtime {
                action: "client_action".into(),
                args: json!({"text":"/infer"}),
            },
        )
        .await,
    );
    wait(&host).await;
    let ui = data(
        command(
            &host,
            w::Command::Runtime {
                action: "extension_ui".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert_eq!(ui["events"][0]["arguments"]["args"][0], "Hello from native");
    assert!(
        server.join().unwrap()["messages"]
            .to_string()
            .contains("extension request")
    );
    assert!(
        !host
            .store
            .artifact_catalog("initial", None, 20, Some("runtime-extension-output"))
            .unwrap()
            .is_empty()
    );
    host.shutdown().await.unwrap();
}

#[test]
fn packaged_jsonl_stdout_is_framed_correlated_and_eof_drains_native_work() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
    };
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let (origin, server) = provider();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    Host::init(&root).unwrap();
    let cfg = dir.path().join("host.json");
    std::fs::write(&cfg, serde_json::to_vec(&config(&origin)).unwrap()).unwrap();
    let mut child = Child(
        Command::new(env!("CARGO_BIN_EXE_ag1-cli"))
            .args([
                "--workspace",
                root.to_str().unwrap(),
                "--state",
                dir.path().join("state").to_str().unwrap(),
                "--config",
                cfg.to_str().unwrap(),
                "--mode",
                "jsonl",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let output = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let first = w::decode(rx.recv_timeout(Duration::from_secs(10)).unwrap().as_bytes()).unwrap();
    let w::WireMessage::Client(view) = first else {
        panic!("initial native view")
    };
    let envelope = w::CommandEnvelope {
        extension: None,
        version: view.snapshot.version,
        scope: view.snapshot.scope,
        expected_revision: view.snapshot.revision,
        id: "jsonl-client-command".to_owned().try_into().unwrap(),
        command: w::Command::Prompt {
            input: w::PromptInput::Inline {
                text: "JSONL fixture".into(),
            },
        },
    };
    let mut input = child.0.stdin.take().unwrap();
    input.write_all(b"invalid JSON\n").unwrap();
    input
        .write_all(&w::encode(&w::WireMessage::Command(envelope)).unwrap())
        .unwrap();
    input.write_all(b"\n").unwrap();
    drop(input);
    let mut framing_error = false;
    let mut acknowledgement = false;
    let mut completed = false;
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(10)) {
        assert!(line.len() <= w::MAX_ENVELOPE);
        match w::decode(line.as_bytes()).expect("stdout is only protocol") {
            w::WireMessage::Error(_) => framing_error = true,
            w::WireMessage::Bridge(w::BridgeFrame::Reply { id, status, .. })
                if id.as_str() == "jsonl-client-command" =>
            {
                assert_eq!(status, "accepted");
                acknowledgement = true;
            }
            w::WireMessage::Client(view)
                if view.snapshot.status == "idle"
                    && view
                        .snapshot
                        .messages
                        .iter()
                        .any(|m| m.text.contains("Hello from native")) =>
            {
                completed = true
            }
            _ => {}
        }
    }
    // Pipe EOF may arrive before the OS publishes the child's exit status.
    // Wait for that distinct event, retaining a bound on shutdown itself.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "EOF completed within the deadline"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(status.success());
    assert!(framing_error && acknowledgement && completed);
    assert!(
        server.join().unwrap()["messages"]
            .to_string()
            .contains("JSONL fixture")
    );
}

#[test]
fn actual_advertised_hooks_export_targets_are_absent_at_the_reference_pin() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../reference/ag1-executable/packages/coding-agent");
    let package: Value =
        serde_json::from_slice(&std::fs::read(root.join("package.json")).unwrap()).unwrap();
    let hooks = &package["exports"]["./hooks"];
    assert!(!hooks.is_null(), "verify actual advertised export");
    fn targets(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::String(s) => out.push(s.clone()),
            Value::Object(map) => {
                for v in map.values() {
                    targets(v, out)
                }
            }
            _ => {}
        }
    }
    let mut paths = vec![];
    targets(hooks, &mut paths);
    assert!(!paths.is_empty());
    for path in paths {
        assert!(!root.join(path).exists());
    }
    assert!(!root.join("src/hooks.ts").exists());
    assert!(!root.join("src/hooks/index.ts").exists());
}

#[tokio::test]
async fn configured_local_package_is_discovered_and_settings_fields_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    Host::init(&root).unwrap();
    std::fs::create_dir_all(root.join(".ag1")).unwrap();
    std::fs::create_dir_all(root.join("package/prompts")).unwrap();
    std::fs::write(
        root.join("package/package.json"),
        r#"{"name":"fixture-resource","ag1":{"prompts":["prompts/review.md"]}}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("package/prompts/review.md"),
        "Review $1 carefully.",
    )
    .unwrap();
    std::fs::write(
        root.join(".ag1/settings.json"),
        r#"{"unrelated":{"keep":0},"packages":[]}"#,
    )
    .unwrap();
    let mut cfg = config("http://127.0.0.1:9");
    cfg.policy_mode = "autonomous".into();
    let host = Host::open(&root, &dir.path().join("private"), cfg)
        .await
        .unwrap();
    for enabled in [true, false] {
        data(command(&host,w::Command::Runtime {action:"resource_action".into(),args:json!({"name":"configurePackage","arguments":{"source":"./package","enabled":enabled}})}).await);
        wait(&host).await;
        let settings: Value =
            serde_json::from_slice(&std::fs::read(root.join(".ag1/settings.json")).unwrap())
                .unwrap();
        assert_eq!(settings["unrelated"]["keep"], 0);
        let resources = data(
            command(
                &host,
                w::Command::Runtime {
                    action: "resources".into(),
                    args: json!({}),
                },
            )
            .await,
        );
        assert_eq!(
            resources["prompts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == "review"),
            enabled
        );
    }
    host.shutdown().await.unwrap();
}

#[test]
fn escaped_long_blocks_still_make_progress_in_bounded_message_pages() {
    let message = json!({"type":"message","id":"large","message":{"role":"assistant","content":(0..8).map(|_|json!({"type":"text","text":"\u{0001}".repeat(10000)})).collect::<Vec<_>>()}});
    let view = ag1_platform::client::projection::message(&message).unwrap();
    assert!(serde_json::to_vec(&view).unwrap().len() < 6000);
    assert!(!view.blocks.is_empty());
}

#[tokio::test]
async fn client_effect_receipts_prevent_duplicate_saves_and_uncertain_replay() {
    let (_dir, host) = host("http://127.0.0.1:9").await;
    std::fs::write(host.workspace.root().join("note.txt"), "original").unwrap();
    let file = data(
        command(
            &host,
            w::Command::Files {
                action: "read".into(),
                args: json!({"path":"note.txt"}),
            },
        )
        .await,
    );
    let mut save = ag1_cli::envelope(&host, "unused").unwrap();
    save.command = w::Command::Files {
        action: "save".into(),
        args: json!({"path":"note.txt","revision":file["revision"],"text":"saved"}),
    };
    let first = host.dispatch(save.clone()).await.unwrap();
    assert!(matches!(&first,w::BridgeFrame::Reply {durable:true,status,..} if status=="completed"));
    std::fs::write(
        host.workspace.root().join("note.txt"),
        "external after save",
    )
    .unwrap();
    assert_eq!(host.dispatch(save.clone()).await.unwrap(), first);
    assert_eq!(
        std::fs::read_to_string(host.workspace.root().join("note.txt")).unwrap(),
        "external after save"
    );
    let status = data(
        command(
            &host,
            w::Command::Runtime {
                action: "client_effect_result".into(),
                args: json!({"command":save.id}),
            },
        )
        .await,
    );
    assert_eq!(status["state"], "completed");
    let mut uncertain = ag1_cli::envelope(&host, "unused").unwrap();
    uncertain.command = w::Command::Files {
        action: "save".into(),
        args: json!({"path":"note.txt","revision":file["revision"],"text":"must not execute"}),
    };
    let key = format!(
        "native-client-{}",
        ag1_storage::digest(
            json!([
                uncertain.scope.workspace,
                uncertain.scope.session,
                uncertain.id
            ])
            .to_string()
            .as_bytes()
        )
    );
    host.store
        .admit_client_command(
            uncertain.scope.session.as_str(),
            &key,
            uncertain.expected_revision.get(),
            &json!({"kind":"client-effect","envelope":uncertain}),
        )
        .unwrap();
    assert!(matches!(
        host.dispatch(uncertain).await,
        Err(ag1_core::DomainError::DurabilityUnknown)
    ));
    assert_eq!(
        std::fs::read_to_string(host.workspace.root().join("note.txt")).unwrap(),
        "external after save"
    );
    host.shutdown().await.unwrap();
}

async fn upload(host: &Arc<Host>, bytes: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let begin = data(
        command(
            host,
            w::Command::Artifacts {
                action: "upload_begin".into(),
                args: json!({}),
            },
        )
        .await,
    );
    let id = &begin["upload"];
    for (index, chunk) in bytes.chunks(32768).enumerate() {
        data(command(host,w::Command::Artifacts {action:"upload_append".into(),args:json!({"upload":id,"offset":(index*32768).to_string(),"base64":STANDARD.encode(chunk)})}).await);
    }
    data(
        command(
            host,
            w::Command::Artifacts {
                action: "upload_finish".into(),
                args: json!({"upload":id,"sha256":ag1_storage::digest(bytes)}),
            },
        )
        .await,
    )["artifact"]
        .as_str()
        .unwrap()
        .into()
}
#[tokio::test]
async fn corrupted_artifact_is_visible_as_failure_and_never_sent_to_provider() {
    let (_dir, host) = host("http://127.0.0.1:9").await;
    let bytes = br#"{"role":"user","content":[{"type":"text","text":"retained"}]}"#;
    let id = upload(&host, bytes).await;
    std::fs::write(
        host.store.blob_path(&ag1_storage::digest(bytes)).unwrap(),
        b"corrupt",
    )
    .unwrap();
    let mut envelope = ag1_cli::envelope(&host, "unused").unwrap();
    envelope.command = w::Command::ArtifactRead {
        artifact: id.clone().try_into().unwrap(),
        offset: w::Sequence::new(0),
        limit: 32768,
    };
    assert!(
        matches!(host.reply(envelope).await,w::BridgeFrame::Reply {status,..} if status=="rejected")
    );
    assert!(
        matches!(command(&host,w::Command::Prompt {input:w::PromptInput::MessageArtifact {artifact:id.try_into().unwrap()}}).await,w::BridgeFrame::Reply {status,..} if status=="rejected")
    );
    host.shutdown().await.unwrap();
}

fn tooling() -> ag1_platform::client::tooling::Tooling {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let rg = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("rg"))
        .find(|p| p.is_file())
        .unwrap();
    ag1_platform::client::tooling::Tooling {
        browser: None,
        process_host: repository.join("product/target/debug/ag1-process-host"),
        git: "/usr/bin/git".into(),
        rg,
        fd: "/usr/bin/fd".into(),
        bubblewrap: "/usr/bin/bwrap".into(),
        targets: ag1_platform::projects::Toolchains {
            node: None,
            npm: None,
            cargo: None,
            rustc: None,
            godot: None,
            mounts: vec![],
        },
    }
}
#[tokio::test]
async fn terminal_drains_without_a_renderer_and_reports_owned_stop() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    Host::init(&root).unwrap();
    let mut cfg = config("http://127.0.0.1:9");
    cfg.tooling = Some(tooling());
    let host = Host::open(&root, &dir.path().join("private"), cfg)
        .await
        .unwrap();
    data(
        command(
            &host,
            w::Command::Runtime {
                action: "terminal_start".into(),
                args: json!({}),
            },
        )
        .await,
    );
    data(
        command(
            &host,
            w::Command::Runtime {
                action: "terminal_input".into(),
                args: json!({"text":"printf 'owned-terminal-result\\n'\n"}),
            },
        )
        .await,
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let terminal = data(
        command(
            &host,
            w::Command::Runtime {
                action: "terminal_status".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert!(
        terminal["text"]
            .as_str()
            .unwrap()
            .contains("owned-terminal-result")
    );
    assert!(terminal["sequence"].as_u64().unwrap() > 0);
    data(
        command(
            &host,
            w::Command::Runtime {
                action: "terminal_stop".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert_eq!(
        data(
            command(
                &host,
                w::Command::Runtime {
                    action: "terminal_status".into(),
                    args: json!({})
                }
            )
            .await
        )["open"],
        false
    );
    let project = data(
        command(
            &host,
            w::Command::Project {
                action: "status".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert_eq!(project["available"], true);
    assert_eq!(project["preview"]["available"], false);
    host.shutdown().await.unwrap();
}
#[test]
fn workflow_text_matches_interactive_steering_and_rejects_unknown_dependencies() {
    use ag1_platform::client::workflow;
    let input = json!({"name":"Review","steps":[{"id":"inspect","type":"explore","prompt":"Read the source"},{"id":"report","type":"verify","prompt":"Report observations","dependsOn":["inspect"]}]});
    assert_eq!(
        workflow::text(input.to_string().as_bytes()).unwrap(),
        "Execute the following workflow \"Review\":\nStep \"inspect\" (explore): Read the source\nStep \"report\" (verify) (depends on: inspect): Report observations\n\nExecute each step using the task tool. Wait for dependent steps to complete before starting their dependents."
    );
    let mut unknown = input.clone();
    unknown["steps"][1]["dependsOn"] = json!(["missing"]);
    assert!(
        format!(
            "{:?}",
            workflow::text(unknown.to_string().as_bytes()).unwrap_err()
        )
        .contains("unknown step")
    );
    let mut cycle = input;
    cycle["steps"][0]["dependsOn"] = json!(["report"]);
    assert!(
        format!(
            "{:?}",
            workflow::text(cycle.to_string().as_bytes()).unwrap_err()
        )
        .contains("Circular")
    );
}

#[tokio::test]
async fn binary_artifact_catalog_and_vision_input_use_verified_custody() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let (origin, server) = provider();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    Host::init(&root).unwrap();
    let mut cfg = config(&origin);
    cfg.models[0]["input"] = json!(["text", "image"]);
    let host = Host::open(&root, &dir.path().join("private"), cfg)
        .await
        .unwrap();
    let png = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/tools/images/small.png"),
    )
    .unwrap();
    let session = host.view().unwrap().snapshot.scope.session;
    let record = ag1_platform::projects::preview::retain(
        &host.store,
        session.as_str(),
        session.as_str(),
        &png,
        "web-visual",
        "client fixture",
        "owned fixture",
    )
    .unwrap();
    let catalog = data(
        command(
            &host,
            w::Command::Artifacts {
                action: "list".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert!(
        catalog["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == record["id"] && r["mime_type"] == "image/png")
    );
    let mut message = json!({"role":"user","content":[{"type":"image","mimeType":"image/jpeg","data":STANDARD.encode(&png)}]});
    let wrong = upload(&host, &serde_json::to_vec(&message).unwrap()).await;
    let mut request = ag1_cli::envelope(&host, "unused").unwrap();
    request.command = w::Command::Prompt {
        input: w::PromptInput::MessageArtifact {
            artifact: wrong.try_into().unwrap(),
        },
    };
    let reply = host.reply(request).await;
    assert!(matches!(reply,w::BridgeFrame::Reply {status,..} if status=="rejected"));
    message["content"][0]["mimeType"] = json!("image/png");
    let id = upload(&host, &serde_json::to_vec(&message).unwrap()).await;
    command(
        &host,
        w::Command::Prompt {
            input: w::PromptInput::MessageArtifact {
                artifact: id.try_into().unwrap(),
            },
        },
    )
    .await;
    wait(&host).await;
    let body = server.join().unwrap();
    assert!(
        body["messages"]
            .to_string()
            .contains("data:image/png;base64,")
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn fork_commits_before_restoring_full_user_text_and_new_scope() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let (origin, server) = provider();
    let (_dir, host) = host(&origin).await;
    let text = "Fork input Ω ".repeat(3500);
    let id = upload(&host, text.as_bytes()).await;
    command(
        &host,
        w::Command::Prompt {
            input: w::PromptInput::Artifact {
                artifact: id.try_into().unwrap(),
            },
        },
    )
    .await;
    wait(&host).await;
    server.join().unwrap();
    let before = host.view().unwrap();
    let entry = before
        .details
        .messages
        .iter()
        .find(|m| m.role == "user")
        .unwrap()
        .id
        .clone();
    let mut bytes = vec![];
    let mut offset = json!("0");
    loop {
        let page = data(
            command(
                &host,
                w::Command::Session {
                    action: "user_text".into(),
                    args: json!({"id":entry,"offset":offset}),
                },
            )
            .await,
        );
        bytes.extend(STANDARD.decode(page["base64"].as_str().unwrap()).unwrap());
        offset = page["next"].clone();
        if offset.is_null() {
            break;
        }
    }
    assert_eq!(String::from_utf8(bytes).unwrap(), text);
    let reply = data(
        command(
            &host,
            w::Command::Session {
                action: "fork".into(),
                args: json!({"entry":entry}),
            },
        )
        .await,
    );
    wait(&host).await;
    let after = host.view().unwrap();
    assert_ne!(before.snapshot.scope.session, after.snapshot.scope.session);
    assert_eq!(
        after.details.activity["last_terminal"]["status"],
        "completed"
    );
    assert_eq!(
        after.details.activity["last_terminal"]["operation"],
        reply["operation"]
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn native_client_runs_and_stops_a_real_web_target_with_scoped_preview() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    Host::init(&root).unwrap();
    for name in ["package.json", "package-lock.json", "index.html", "main.js"] {
        std::fs::copy(
            repository.join("fixtures/projects/web").join(name),
            root.join(name),
        )
        .unwrap();
    }
    let node = repository.join(".local/toolchains/node-v24.14.0-linux-x64");
    let mut tools = tooling();
    tools.targets.node = Some(node.join("bin/node"));
    tools.targets.npm = Some(node.join("lib/node_modules/npm/bin/npm-cli.js"));
    tools.targets.mounts.push(node);
    let mut cfg = config("http://127.0.0.1:9");
    cfg.tooling = Some(tools);
    let host = Host::open(&root, &dir.path().join("private"), cfg)
        .await
        .unwrap();
    async fn status(host: &Arc<Host>) -> Value {
        for _ in 0..100 {
            let mut envelope = ag1_cli::envelope(host, "unused").unwrap();
            envelope.command = w::Command::Project {
                action: "status".into(),
                args: json!({}),
            };
            match host.dispatch(envelope).await {
                Ok(frame) => return data(frame),
                Err(ag1_core::DomainError::QueueFull) => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(error) => panic!("{error:?}"),
            }
        }
        panic!("runtime remained busy")
    }
    let initial = status(&host).await;
    let target = initial["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "web")
        .unwrap()["id"]
        .clone();
    for action in ["install", "run"] {
        let state = status(&host).await;
        let started = data(
            command(
                &host,
                w::Command::Project {
                    action: "start".into(),
                    args: json!({"target":target,"operation":action,"revision":state["revision"]}),
                },
            )
            .await,
        );
        let id = started["operation"].clone();
        let finished = tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let state = status(&host).await;
                let record = state["history"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["id"] == id)
                    .unwrap();
                assert_ne!(record["lifecycle"], "failed", "{state}");
                if record["lifecycle"] == if action == "run" { "ready" } else { "exited" } {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;
        if finished.is_err() {
            let state = status(&host).await;
            let log = data(
                command(
                    &host,
                    w::Command::Project {
                        action: "log".into(),
                        args: json!({"operation":id,"offset":"0","stream":0}),
                    },
                )
                .await,
            );
            panic!("{action}: {state}; {log}");
        }
    }
    let scope = host.view().unwrap().snapshot.scope;
    let grant = host
        .with_project(&scope, |runtime| runtime.preview_grant())
        .unwrap();
    assert!(grant.origin.starts_with("http://127.0.0.1:"));
    let mut stale = scope.clone();
    stale.session = "other".to_owned().try_into().unwrap();
    assert!(
        host.with_project(&stale, |runtime| runtime.preview_grant())
            .is_err()
    );
    data(
        command(
            &host,
            w::Command::Project {
                action: "stop".into(),
                args: json!({}),
            },
        )
        .await,
    );
    assert!(
        !status(&host).await["preview"]["available"]
            .as_bool()
            .unwrap()
    );
    assert!(
        host.with_project(&scope, |runtime| runtime.verify_preview(&grant))
            .is_err()
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn configured_browser_host_registers_visual_schemas_without_starting_a_preview() {
    for browser_available in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        Host::init(&root).unwrap();
        let mut tools = tooling();
        // Admission only: these fixture paths satisfy discovery, and are never
        // executed. Existing target tests cover actual browser execution.
        if browser_available {
            let module = dir.path().join("unused-browser-module.js");
            std::fs::write(&module, "throw new Error('must not execute');").unwrap();
            tools.browser = Some(ag1_platform::client::tooling::Browser {
                node: "/usr/bin/true".into(),
                driver: module.clone(),
                playwright: module,
                executable: "/usr/bin/true".into(),
            });
        }
        let mut cfg = config("http://127.0.0.1:9");
        cfg.tooling = Some(tools);
        let state = dir.path().join("private");
        for _ in 0..2 {
            let host = Host::open(&root, &state, cfg.clone()).await.unwrap();
            let selected = host.registry.selected().unwrap();
            let catalog = selected.services.tools().client_catalog();
            let active = selected.services.runtime().configuration().unwrap().tools;
            for name in ["screenshot_url", "interact_then_screenshot", "verify_ui"] {
                assert_eq!(
                    catalog.iter().any(|tool| tool["name"] == name),
                    browser_available
                );
                assert_eq!(
                    active.iter().any(|tool| tool["name"] == name),
                    browser_available
                );
            }
            assert!(
                selected
                    .services
                    .runtime()
                    .pending_transport()
                    .unwrap()
                    .is_none()
            );
            host.shutdown().await.unwrap();
            drop(host);
            let record = ag1_platform::cutover::Registry::for_user()
                .unwrap()
                .inspect(&root)
                .unwrap()
                .unwrap();
            assert!(record.quiescent);
        }
    }
}
