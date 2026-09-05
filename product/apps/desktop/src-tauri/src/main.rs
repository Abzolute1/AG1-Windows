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
async fn open_project(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    path: String,
) -> Result<Value, String> {
    local(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.open(&path))
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
            bridge_command,
            bridge_view,
            catalog,
            save_preferences,
            quit
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<Arc<Desktop>>();
                let active = state.worker.lock().unwrap().as_ref().is_some_and(|w| {
                    let view = w.view();
                    view["snapshot"]["status"] != "idle" && view["error"].is_null()
                });
                if active {
                    api.prevent_close();
                    let _ = window.emit("close-requested", ());
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Desktop launch failed");
}
