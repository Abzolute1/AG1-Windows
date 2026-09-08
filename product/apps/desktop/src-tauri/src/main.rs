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
async fn bridge_command(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    envelope: CommandEnvelope,
) -> Result<Value, String> {
    local(&window)?;
    let state = state.inner().clone();
    if state.native.enabled() {
        if let ag1_protocol::Command::Project { action, .. } = &envelope.command
            && action.starts_with("preview_")
        {
            return state.native.preview(window.app_handle(), envelope).await;
        }
        let host = state.native.host().await?;
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
fn save_preferences(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    theme: String,
) -> Result<(), String> {
    local(&window)?;
    if !["light", "dark", "system"].contains(&theme.as_str()) {
        return Err("Invalid theme".into());
    }
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
    state.native.shutdown().await?;
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(worker) = state.worker.lock().unwrap().as_mut() {
            worker.stop();
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}
fn main() {
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
            #[cfg(feature = "visual-test")]
            visual_test::setup(app)?;
            #[cfg(not(feature = "visual-test"))]
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_project,
            cutover_status,
            bridge_command,
            bridge_view,
            catalog,
            save_preferences,
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
