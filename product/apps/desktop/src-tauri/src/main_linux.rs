#[cfg(target_os = "linux")]
mod github_secret_entry;
#[cfg(target_os = "linux")]
mod mcp_input_entry;
#[cfg(target_os = "linux")]
mod mcp_oauth_entry;
#[cfg(target_os = "linux")]
mod mcp_private_entry;
#[cfg(target_os = "linux")]
mod rendering;
#[cfg(feature = "visual-test")]
mod visual_test;
use ag1_desktop::{Desktop, file_operation};
use ag1_protocol::CommandEnvelope;
use serde_json::{Value, json};
use std::{io::Read, path::PathBuf, sync::Arc};
use tauri::{Emitter, Manager};
fn local(window: &tauri::Window) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("This window has no agent capability".into())
    }
}
#[tauri::command]
async fn bridge_live(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    after: u64,
) -> Result<Value, String> {
    local(&window)?;
    let host = state.native.host().await?;
    host.live_display(after).await.map_err(|e| format!("{e:?}"))
}
#[tauri::command]
async fn github_command(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    action: String,
    args: Value,
) -> Result<Value, String> {
    local(&window)?;
    let state = state.inner().clone();
    if action == "api_schema" {
        return ag1_platform::github::schemas::get(args["id"].as_str().unwrap_or(""));
    }
    if action == "api_search" {
        return ag1_platform::github::schemas::search(
            args["query"].as_str().unwrap_or(""),
            args["page"].as_u64().unwrap_or(0) as usize,
        );
    }
    if action == "api_cancel" {
        state.github.shared(&state.state)?.cancel();
        return Ok(json!({"cancelled":true}));
    }
    if matches!(
        action.as_str(),
        "capabilities"
            | "api_read"
            | "api_preview"
            | "api_confirm"
            | "api_dismiss"
            | "repos"
            | "issues"
            | "pulls"
            | "runs"
            | "files"
            | "item"
            | "pull_review"
    ) {
        if action == "capabilities" {
            return Ok(ag1_platform::github::operations::catalog());
        }
        let _transition = state.transition.lock().await;
        let scope = match state.native.host().await {
            Ok(host) => host.github_scope().map_err(|e| format!("{e:?}"))?,
            // Browsing an account does not require a working folder. Opening a
            // conversation changes this native scope and invalidates previews.
            Err(_) => serde_json::json!(["desktop-account", state.state]).to_string(),
        };
        let service = state.github.shared(&state.state)?;
        return tauri::async_runtime::spawn_blocking(move || {
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let control = ag1_platform::resource_fs::ScanControl {
                cancel: &cancel,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
            };
            if [
                "repos",
                "issues",
                "pulls",
                "runs",
                "files",
                "item",
                "pull_review",
            ]
            .contains(&action.as_str())
            {
                return ag1_platform::github::browse::read(
                    &service, &action, &args, &scope, &control,
                )
                .map_err(|error| error.message.to_string());
            }
            let result = match action.as_str() {
                "api_read" => {
                    let request = serde_json::from_value(args)
                        .map_err(|_| "Invalid GitHub read operation".to_string())?;
                    service
                        .read(vec![request], &scope, &control)
                        .map(|mut replies| serde_json::to_value(replies.remove(0)).unwrap())
                }
                "api_preview" => {
                    let request = serde_json::from_value(args)
                        .map_err(|_| "Invalid GitHub operation".to_string())?;
                    service.preview(request, &scope, &control)
                }
                "api_confirm" => service
                    .confirm(
                        args["confirmation"].as_str().unwrap_or(""),
                        &scope,
                        &control,
                    )
                    .map(|reply| serde_json::to_value(reply).unwrap()),
                _ => {
                    service.dismiss(args["confirmation"].as_str().unwrap_or(""));
                    Ok(json!({"dismissed":true}))
                }
            };
            Ok(match result {
                Ok(value) => json!({"ok":true,"value":value}),
                Err(error) => json!({"ok":false,"error":error}),
            })
        })
        .await
        .map_err(|_| "GitHub action interrupted".to_string())?;
    }
    if matches!(action.as_str(), "checkout" | "push") {
        let _transition = state.transition.lock().await;
        let host = state.native.host().await?;
        let _guard = host
            .idle_client_guard()
            .await
            .map_err(|e| format!("{e:?}"))?;
        let workspace = Some(
            host.working_folder(true)
                .map_err(|e| format!("{e:?}"))?
                .root()
                .to_owned(),
        );
        let state = state.clone();
        return tauri::async_runtime::spawn_blocking(move || {
            state.github.request(&state.state, &action, args, workspace)
        })
        .await
        .map_err(|_| "GitHub action interrupted".to_string())?;
    }
    let workspace = if action == "local" {
        Some(
            state
                .native
                .host()
                .await?
                .working_folder(false)
                .map_err(|e| format!("{e:?}"))?
                .root()
                .to_owned(),
        )
    } else {
        None
    };
    tauri::async_runtime::spawn_blocking(move || {
        state.github.request(&state.state, &action, args, workspace)
    })
    .await
    .map_err(|_| "GitHub action interrupted".to_string())?
}
#[tauri::command]
async fn choose_chat_folder(window: tauri::Window) -> Result<Option<String>, String> {
    local(&window)?;
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let parent = window.clone();
        window
            .run_on_main_thread(move || {
                let parent = match parent.gtk_window() {
                    Ok(parent) => parent,
                    Err(error) => {
                        let _ = sender.send(Err(error.to_string()));
                        return;
                    }
                };
                let dialog = gtk::FileChooserNative::new(
                    Some("Choose a workspace folder"),
                    Some(&parent),
                    gtk::FileChooserAction::SelectFolder,
                    Some("Choose folder"),
                    Some("Cancel"),
                );
                dialog.set_modal(true);
                dialog.set_create_folders(true);
                // Native dialogs have no window-system reference: keep ownership
                // across the asynchronous response instead of dropping after show().
                gtk::glib::MainContext::default().spawn_local(async move {
                    let response = dialog.run_future().await;
                    let value = if response == gtk::ResponseType::Accept {
                        dialog.filename().map(|p| p.to_string_lossy().into_owned())
                    } else {
                        None
                    };
                    let _ = sender.send(Ok(value));
                    dialog.destroy();
                });
            })
            .map_err(|e| e.to_string())?;
        receiver
            .await
            .map_err(|_| "Folder picker closed unexpectedly; enter the path instead".to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    Err("Enter the folder path on this platform".into())
}
#[tauri::command]
async fn cutover_status(window: tauri::Window, path: String) -> Result<Value, String> {
    local(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let record = ag1_platform::cutover::Registry::for_user()
            .and_then(|registry| registry.inspect(std::path::Path::new(&path)))
            .map_err(|e| format!("{e:?}"))?;
        serde_json::to_value(record).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn open_project(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    path: String,
) -> Result<Value, String> {
    local(&window)?;
    #[cfg(feature = "visual-test")]
    if let Some(selected) = std::env::var_os("AG1_CUTOVER_WORKSPACE")
        && std::fs::canonicalize(&path).map_err(|e| e.to_string())?
            != std::fs::canonicalize(selected).map_err(|e| e.to_string())?
    {
        return Err("Cutover measurement is bound to its selected workspace".into());
    }
    let state = state.inner().clone();
    let _transition = state.transition.lock().await;
    if state.native.selects(std::path::Path::new(&path))? {
        {
            let mut slot = state.worker.lock().map_err(|_| "Worker lock")?;
            if let Some(worker) = slot.as_ref()
                && worker.view()["snapshot"]["status"] != "idle"
                && worker.view()["error"].is_null()
            {
                return Err("Cancel and drain active work before switching projects".into());
            }
            if let Some(worker) = slot.as_mut() {
                worker.stop();
            }
            *slot = None;
        }
        let result = state
            .native
            .open(std::path::Path::new(&path), &state.state)
            .await?;
        state.add_project(std::path::Path::new(&path))?;
        return Ok(result);
    }
    if state.native.enabled() {
        let view = state.native.view().await?;
        if !view["snapshot"].is_null() && view["snapshot"]["status"] != "idle" {
            return Err("Cancel and drain active work before switching projects".into());
        }
        state.native.shutdown().await?;
    }
    let worker_state = state.clone();
    tauri::async_runtime::spawn_blocking(move || worker_state.open(&path))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
fn desktop_preferences(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    Ok(state.native.preferences())
}
#[tauri::command]
fn desktop_models(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    Ok(state.native.models())
}
#[tauri::command]
async fn open_general(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    let _transition = state.transition.lock().await;
    if let Some(worker) = state.worker.lock().map_err(|_| "Worker lock")?.as_ref() {
        if worker.view()["snapshot"]["status"] != "idle" {
            return Err("Stop active work before switching chats".into());
        }
    }
    if let Some(mut worker) = state.worker.lock().map_err(|_| "Worker lock")?.take() {
        worker.stop();
    }
    state.native.general(&state.state).await
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderingSample {
    frames: u32,
    #[serde(default)]
    mean_ms: f64,
    median_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    over_25_ms: u32,
    width: u32,
    height: u32,
}
#[tauri::command]
async fn record_rendering_sample(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    sample: RenderingSample,
) -> Result<(), String> {
    local(&window)?;
    if !(12..=180).contains(&sample.frames)
        || sample.over_25_ms > sample.frames
        || [
            sample.mean_ms,
            sample.median_ms,
            sample.p95_ms,
            sample.max_ms,
        ]
        .iter()
        .any(|n| !n.is_finite() || *n < 0.0 || *n > 60000.0)
        || sample.median_ms > sample.p95_ms
        || sample.p95_ms > sample.max_ms
        || sample.width > 32768
        || sample.height > 32768
    {
        return Err("Invalid frame sample".into());
    }
    let path = state.state.join("rendering-samples.jsonl");
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        let old = if std::fs::metadata(&path).is_ok_and(|m| m.len() <= 65536) { std::fs::read_to_string(&path).unwrap_or_default() } else { String::new() };
        let mut rows: Vec<_> = old.lines().rev().take(19).collect(); rows.reverse();
        let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or("State directory")?).map_err(|e|e.to_string())?;
        for row in rows { writeln!(file,"{row}").map_err(|e|e.to_string())?; }
        writeln!(file,"{}",serde_json::json!({"sample":sample,"recorded_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()})).map_err(|e|e.to_string())?;
        file.persist(path).map_err(|e|e.to_string())?; Ok(())
    }).await.map_err(|e|e.to_string())?
}
#[tauri::command]
async fn select_conversation(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    session: Option<String>,
) -> Result<Value, String> {
    local(&window)?;
    let _transition = state.transition.lock().await;
    state.native.select_conversation(session.as_deref()).await
}
#[tauri::command]
async fn conversation_statuses(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    state.native.conversation_statuses().await
}
#[tauri::command]
fn save_chat(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    chat: Value,
) -> Result<(), String> {
    local(&window)?;
    state.save_chat(chat)
}
#[tauri::command]
async fn model_reasoning(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    provider: String,
    id: String,
) -> Result<Value, String> {
    local(&window)?;
    state.native.reasoning_controls(&provider, &id).await
}
#[tauri::command]
async fn bridge_command(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    mut envelope: CommandEnvelope,
) -> Result<Value, String> {
    local(&window)?;
    let state = state.inner().clone();
    if state.native.enabled() {
        if let ag1_protocol::Command::Project { action, .. } = &envelope.command
            && action.starts_with("preview_")
        {
            return state.native.preview(window.app_handle(), envelope).await;
        }
        let host = state.native.host_for(&envelope.scope).await?;
        if let ag1_protocol::Command::Model { action, args } = &envelope.command
            && action == "reasoning"
        {
            let provider = args["provider"].as_str().ok_or("Missing model provider")?;
            let id = args["model_id"].as_str().ok_or("Missing model identity")?;
            let mode = args["mode"].as_str().ok_or("Missing reasoning mode")?;
            let view = serde_json::to_value(host.view().map_err(|e| format!("{e:?}"))?)
                .map_err(|e| e.to_string())?;
            if view["snapshot"]["model"] != id
                || view["details"]["context"]["selected_model"]["provider"] != provider
            {
                return Err("Model changed before reasoning could be applied".into());
            }
            let profile = state.native.reasoning_controls(provider, id).await?;
            let selected = profile["modes"].as_array().and_then(|modes| modes.iter().find(|m| m["id"] == mode)).ok_or("The backend cannot confirm this reasoning setting. Refresh the model picker before retrying.")?;
            envelope.command = ag1_protocol::Command::Model {
                action: "thinking".into(),
                args: json!({"level":selected["level"],"native_profile":if (profile["source"] == "llama-server" || profile["apply"] == "reasoning_effort") && profile["kind"] == "levels" {json!({"apply":"reasoning_effort","modes":profile["modes"].as_array().unwrap().iter().map(|m|json!({"id":m["id"],"nativeValue":m.get("nativeValue").unwrap_or(&m["id"])})).collect::<Vec<_>>()})} else {Value::Null}}),
            };
        }
        let result = host.reply(envelope).await;

        return serde_json::to_value(result).map_err(|e| e.to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let worker = state.worker.lock().unwrap();
        let result = worker
            .as_ref()
            .ok_or("Open a project first")?
            .command(envelope)?;
        serde_json::to_value(result).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn bridge_view(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    let state = state.inner().clone();
    if state.native.enabled() {
        return state.native.view().await;
    }
    tauri::async_runtime::spawn_blocking(move || {
        Ok(state
            .worker
            .lock()
            .unwrap()
            .as_ref()
            .map(|w| w.view())
            .unwrap_or(json!({"snapshot":null,"error":null,"progress":""})))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn catalog(window: tauri::Window, state: tauri::State<'_, Arc<Desktop>>) -> Result<Value, String> {
    local(&window)?;
    state.catalog()
}
#[tauri::command]
fn export_theme(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    plugin: Value,
) -> Result<String, String> {
    local(&window)?;
    state.export_theme(plugin)
}
#[tauri::command]
fn import_theme(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    plugin: Value,
) -> Result<(), String> {
    local(&window)?;
    state.import_theme(plugin)
}
#[tauri::command]
fn remove_theme(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    id: String,
) -> Result<(), String> {
    local(&window)?;
    state.remove_theme(&id)
}
#[tauri::command]
fn save_preferences(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    theme: String,
) -> Result<(), String> {
    local(&window)?;
    state.set_theme(&theme)
}
#[tauri::command]
async fn quit(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    local(&window)?;
    let state = state.inner().clone();
    state.github.shutdown();
    // Never make the window hostage to a dead provider stream. Native hosts
    // and the legacy worker are given bounded best-effort shutdown time, then
    // the application exits regardless of backend health.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(12), state.native.shutdown()).await;
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        tauri::async_runtime::spawn_blocking(move || {
            let _ = state.github.wait_stopped();
            if let Some(worker) = state.worker.lock().unwrap().as_mut() {
                worker.stop();
            }
        }),
    )
    .await;
    app.exit(0);
    Ok(())
}
pub fn run() {
    #[cfg(target_os = "linux")]
    rendering::configure_environment();
    #[cfg(feature = "visual-test")]
    if std::env::args().nth(1).as_deref() == Some("--p14-cancellation-fixture") {
        ag1_desktop::cancellation_fixture::serve().expect("Controlled cancellation fixture");
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--file-operation") {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(65537)
            .read_to_end(&mut bytes)
            .unwrap();
        let result = if bytes.len() > 65536 {
            Err("Native envelope too large".into())
        } else {
            serde_json::from_slice(&bytes)
                .map_err(|e| e.to_string())
                .and_then(file_operation)
        };
        match result {
            Ok(value) => println!("{value}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        }
        return;
    }
    let root = std::env::var_os("AG1_INSTALL_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../..")
                .canonicalize()
                .unwrap()
        });
    let state = std::env::var_os("AG1_DESKTOP_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".local/desktop"));
    let desktop = Arc::new(
        Desktop::new(
            root,
            state,
            std::env::var("AG1_FIXTURE_PROVIDER").as_deref() == Ok("1"),
        )
        .expect("Private desktop state unavailable"),
    );
    tauri::Builder::default()
        .manage(desktop)
        .setup(|app| {
            #[cfg(target_os = "linux")]
            {
                rendering::setup(app)?;
                #[cfg(feature = "visual-test")]
                visual_test::configure_mcp_tls()?;
                let desktop = app.state::<Arc<Desktop>>();
                let window = app
                    .get_webview_window("main")
                    .ok_or("Main window unavailable")?;
                desktop
                    .native
                    .set_mcp_oauth_entry(Arc::new(mcp_oauth_entry::Entry::new(window.clone())))?;
                desktop
                    .native
                    .set_mcp_input_entry(Arc::new(mcp_input_entry::Entry::new(window.clone())))?;
                desktop
                    .native
                    .set_mcp_private_entry(Arc::new(mcp_private_entry::Entry::new(
                        window.clone(),
                    )))?;
                desktop
                    .github
                    .shared(&desktop.state)?
                    .set_private_entry(Arc::new(github_secret_entry::Entry(window)))
                    .map_err(|e| e.message)?;
            }
            #[cfg(feature = "visual-test")]
            visual_test::setup(app)?;
            #[cfg(not(feature = "visual-test"))]
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_project,
            open_general,
            record_rendering_sample,
            select_conversation,
            conversation_statuses,
            desktop_models,
            model_reasoning,
            desktop_preferences,
            github_command,
            choose_chat_folder,
            save_chat,
            cutover_status,
            bridge_command,
            bridge_view,
            bridge_live,
            catalog,
            save_preferences,
            import_theme,
            export_theme,
            remove_theme,
            quit
        ])
        .on_window_event(|window, event| {
            // Preview windows close independently; only main owns application shutdown.
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<Arc<Desktop>>();
                let active = state.worker.lock().unwrap().as_ref().is_some_and(|w| {
                    let view = w.view();
                    view["snapshot"]["status"] != "idle" && view["error"].is_null()
                });
                if active || state.native.enabled() {
                    api.prevent_close();
                    let _ = window.emit("close-requested", ());
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Desktop launch failed");
}
