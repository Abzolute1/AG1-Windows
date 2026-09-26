use ag1_cli::{Result, envelope};
use ag1_platform::client::{Config, Host};
use ag1_protocol as w;
use std::{
    fs,
    io::{BufRead, Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};
fn write(out: &Arc<Mutex<std::io::Stdout>>, frame: &w::WireMessage) -> Result<()> {
    let bytes = w::encode(frame).map_err(|e| format!("{e:?}"))?;
    write_encoded(out, &bytes)
}
fn write_encoded(out: &Arc<Mutex<std::io::Stdout>>, bytes: &[u8]) -> Result<()> {
    let mut out = out.lock().map_err(|_| "stdout unavailable")?;
    out.write_all(bytes)
        .and_then(|_| out.write_all(b"\n"))
        .and_then(|_| out.flush())
        .map_err(|e| e.to_string())
}
#[tokio::main]
async fn main() {
    match run().await {
        Ok(code) => std::process::exit(code as i32),
        Err(e) => {
            eprintln!("ag1: {e}");
            std::process::exit(1);
        }
    }
}
async fn run() -> Result<u8> {
    let mut args = std::env::args().skip(1);
    let mut workspace = None;
    let mut state = None;
    let mut config = None;
    let mut mode = "jsonl".to_string();
    let mut prompt = Vec::new();
    let mut selected = None;
    let mut provider = None;
    let mut thinking = None;
    let mut selected_tools = None;
    let mut session = None;
    let mut fork = None;
    let mut system = None;
    let mut append_system = None;
    let mut max_turns = None;
    let mut trust = false;
    let mut export = None;
    let mut list_models = false;
    let mut subscription_session = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "ag1-cli --init NEW_WORKSPACE\nag1-cli --workspace ADMITTED_WORKSPACE --state PRIVATE_STATE --config HOST_CONFIG [--mode jsonl|print] [PROMPT]\nJSONL: versioned Command/Cursor frames; stdout contains only protocol frames. EOF drains accepted work; approval waits require a connected client."
                );
                return Ok(0);
            }
            "--version" => {
                println!("ag1-cli {}", env!("CARGO_PKG_VERSION"));
                return Ok(0);
            }
            "--init" => {
                let path = args.next().ok_or("--init requires a new path")?;
                Host::init(&PathBuf::from(path)).map_err(|e| format!("{e:?}"))?;
                return Ok(0);
            }
            "--workspace" | "--cwd" => {
                workspace = Some(PathBuf::from(
                    args.next().ok_or("--workspace requires a path")?,
                ))
            }
            "--state" | "--agent-dir" | "--session-dir" => {
                state = Some(PathBuf::from(args.next().ok_or("--state requires a path")?))
            }
            "--config" => {
                config = Some(PathBuf::from(
                    args.next().ok_or("--config requires a path")?,
                ))
            }
            "--mode" => mode = args.next().ok_or("--mode requires jsonl or print")?,
            "--print" | "-p" => mode = "print".into(),
            "--json" | "--rpc" => mode = "jsonl".into(),
            "--model" => selected = Some(args.next().ok_or("--model requires an ID")?),
            "--provider" => provider = Some(args.next().ok_or("--provider requires an ID")?),
            "--thinking" => thinking = Some(args.next().ok_or("--thinking requires a level")?),
            "--tools" => {
                selected_tools = Some(
                    args.next()
                        .ok_or("--tools requires names")?
                        .split(',')
                        .map(str::to_owned)
                        .collect::<Vec<_>>(),
                )
            }
            "--no-tools" => selected_tools = Some(vec![]),
            "--session" | "--resume" => {
                session = Some(
                    args.next()
                        .ok_or("--session/--resume requires a native session ID")?,
                )
            }
            "--continue" | "-c" => (),
            "--fork" => fork = Some(args.next().ok_or("--fork requires an entry ID")?),
            "--system-prompt" => system = Some(args.next().ok_or("--system-prompt requires text")?),
            "--append-system-prompt" => {
                append_system = Some(args.next().ok_or("--append-system-prompt requires text")?)
            }
            "--max-turns" => {
                max_turns = Some(
                    args.next()
                        .ok_or("--max-turns requires a positive count")?
                        .parse::<u64>()
                        .map_err(|_| "Invalid --max-turns")?,
                )
            }
            "--trust-workspace" => trust = true,
            "--export" => {
                export = Some(PathBuf::from(
                    args.next().ok_or("--export requires a new output path")?,
                ))
            }
            "--list-models" => list_models = true,
            "--discover-openai-subscription" => subscription_session = Some(PathBuf::from(args.next().ok_or("--discover-openai-subscription requires the private Codex session file")?)),
            "--" => {
                prompt.extend(args);
                break;
            }
            _ if arg.starts_with('-') => return Err(format!("Unsupported argument: {arg}")),
            _ => prompt.push(arg),
        }
    }
    mode = match mode.as_str() {
        "text" => "print".into(),
        "json" | "rpc" => "jsonl".into(),
        _ => mode,
    };
    if !["print", "jsonl"].contains(&mode.as_str()) {
        return Err("--mode requires jsonl or print".into());
    }
    if let Some(path) = subscription_session {
        let models = ag1_platform::provider::codex_subscription::discover(&path).await.map_err(|e| format!("{e:?}"))?;
        println!("{}", serde_json::to_string(&models).map_err(|e| e.to_string())?);
        return Ok(0);
    }
    let path = config.ok_or("--config is required; no implicit user profile writes")?;
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("host config too large".into());
    }
    let mut config: Config =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid host configuration")?;
    if let Some(system) = system {
        config.system_prompt = system;
    }
    if let Some(append) = append_system {
        config.system_prompt.push('\n');
        config.system_prompt.push_str(&append);
    }
    if max_turns == Some(0) {
        return Err("--max-turns must be positive".into());
    }
    if max_turns.is_some() {
        config.max_generations = max_turns;
    }
    if mode == "print" {
        config.policy_mode = if trust { "autonomous" } else { "headless" }.into();
    }
    if list_models {
        println!("{}",serde_json::to_string(&config.models.iter().map(|m|serde_json::json!({"id":m["id"],"provider":m["provider"],"runtime":m["runtime"]})).collect::<Vec<_>>()).map_err(|e|e.to_string())?);
        return Ok(0);
    }
    let selected = selected
        .map(|id| {
            config
                .models
                .iter()
                .find(|m| m["id"] == id && provider.as_ref().is_none_or(|p| m["provider"] == *p))
                .cloned()
                .ok_or_else(|| "Requested model/provider is not in the host catalog".to_string())
        })
        .transpose()?;
    let host = Host::open(
        &workspace.ok_or("--workspace is required")?,
        &state.ok_or("--state is required")?,
        config,
    )
    .await
    .map_err(|e| format!("{e:?}"))?;
    let mut controls = Vec::new();
    if let Some(session) = session {
        controls.push(w::Command::Session {
            action: "resume".into(),
            args: serde_json::json!({"session":session}),
        });
    }
    if let Some(entry) = fork {
        controls.push(w::Command::Session {
            action: "fork".into(),
            args: serde_json::json!({"entry":entry}),
        });
    }
    if let Some(model) = selected {
        controls.push(w::Command::Model {
            action: "select".into(),
            args: serde_json::json!({"provider":model["provider"],"model_id":model["id"]}),
        });
    }
    if let Some(level) = thinking {
        controls.push(w::Command::Model {
            action: "thinking".into(),
            args: serde_json::json!({"level":level}),
        });
    }
    if let Some(names) = selected_tools {
        controls.push(w::Command::Runtime {
            action: "tools".into(),
            args: serde_json::json!({"names":names}),
        });
    }
    for command in controls {
        let mut request = envelope(&host, "unused")?;
        request.command = command;
        let reply = host.dispatch(request).await.map_err(|e| format!("{e:?}"))?;
        if let w::BridgeFrame::Reply { status, data, .. } = reply
            && status == "rejected"
        {
            return Err(data.to_string());
        }
        loop {
            let view = host.settled_view().await.map_err(|e| format!("{e:?}"))?;
            if view.snapshot.status == "idle" {
                break;
            }
            if view.error.is_some() {
                return Err("Startup control requires recovery".into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
    if let Some(path) = export {
        let html = ag1_platform::client::export::full_html(
            &host
                .registry
                .selected()
                .map_err(|e| format!("{e:?}"))?
                .services
                .runtime()
                .client_records()
                .map_err(|e| format!("{e:?}"))?,
        )
        .map_err(|e| format!("{e:?}"))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(html.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        host.shutdown().await.map_err(|e| format!("{e:?}"))?;
        return Ok(0);
    }
    if mode == "print" {
        if prompt.is_empty() {
            return Err("print requires a prompt".into());
        }
        let result = ag1_cli::print(&host, &prompt.join(" ")).await;
        host.shutdown().await.map_err(|e| format!("{e:?}"))?;
        let (text, code) = result?;
        println!("{text}");
        return Ok(code);
    }
    let out = Arc::new(Mutex::new(std::io::stdout()));
    write(
        &out,
        &w::WireMessage::Client(Box::new(
            host.settled_view().await.map_err(|e| format!("{e:?}"))?,
        )),
    )?;
    if !prompt.is_empty() {
        let result = host
            .dispatch(envelope(&host, &prompt.join(" "))?)
            .await
            .map_err(|e| format!("{e:?}"))?;
        write(&out, &w::WireMessage::Bridge(result))?;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    std::thread::spawn(move || {
        let input = std::io::stdin();
        let mut input = input.lock();
        loop {
            let mut line = Vec::new();
            match input
                .by_ref()
                .take(w::MAX_ENVELOPE as u64 + 2)
                .read_until(b'\n', &mut line)
            {
                Ok(0) => break,
                Ok(_) => {
                    let incomplete = line.last() != Some(&b'\n') && line.len() > w::MAX_ENVELOPE;
                    if tx.blocking_send(line).is_err() || incomplete {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(16));
    let mut last = Vec::new();
    let mut eof = false;
    loop {
        tokio::select! {
            line=rx.recv(),if !eof=>match line {
                None=>eof=true,
                Some(line)=>match w::decode(&line) {
                    Ok(w::WireMessage::Command(command))=>{
                        let frame=w::WireMessage::Bridge(host.reply(command).await);write(&out,&frame)?;
                    },
                    Ok(w::WireMessage::Cursor(cursor))=>{let frame=host.commands.replay(&cursor).map(w::WireMessage::Delivery).unwrap_or_else(|_|w::WireMessage::Error(w::ProtocolError::StaleContext));write(&out,&frame)?;},
                    Ok(_)=>write(&out,&w::WireMessage::Error(w::ProtocolError::Malformed {detail:"Expected command or cursor".into()}))?,
                    Err(error)=>write(&out,&w::WireMessage::Error(error))?,
                }
            },
            _=tick.tick()=>{
                let view=host.settled_view().await.map_err(|e|format!("{e:?}"))?;
                // Compare the exact encoded frame and reuse it for output. A
                // changed view needs neither a second encoding nor a deep clone.
                let frame = w::WireMessage::Client(Box::new(view));
                let serialized = w::encode(&frame).map_err(|e| format!("{e:?}"))?;
                if serialized != last { write_encoded(&out, &serialized)?; last = serialized; }
                let w::WireMessage::Client(view) = frame else { unreachable!("constructed client frame") };
                if eof {
                    if !view.snapshot.pending.is_empty() {host.shutdown().await.map_err(|e|format!("{e:?}"))?;return Err("Input closed with pending approval; cancelled owned work".into());}
                    if view.snapshot.status=="idle" {if let Some(error)=view.error {host.shutdown().await.map_err(|e|format!("{e:?}"))?;return Err(error);}break;}
                    if view.error.is_some() || matches!(view.snapshot.status.as_str(),"recovery_required"|"persistence_failure") {return Err("Native runtime requires recovery".into());}
                }
            }
        }
    }
    host.shutdown().await.map_err(|e| format!("{e:?}"))?;
    Ok(0)
}
