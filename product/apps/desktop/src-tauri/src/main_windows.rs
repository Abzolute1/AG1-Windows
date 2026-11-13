//! Tauri commands for the Windows-native local chat host.

use ag1_core::runtime::planning::PlanRef;
use ag1_desktop::{
    native_host::{NativeHost, NativeModelSetup},
    themes,
};
use ag1_protocol::{Command, CommandEnvelope, PromptInput};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::Manager;
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows_sys::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST,
    OPENFILENAMEW,
};
use windows_sys::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW,
};

struct Desktop {
    host: NativeHost,
    // GitHub credentials remain in GitHub CLI's own store. AG1W retains only
    // connection state and exact, expiring request confirmations.
    github: Arc<ag1_platform::github::GitHub>,
    transition: tokio::sync::Mutex<()>,
    // Sidebar metadata is deliberately separate from the durable message
    // journal. A damaged sidebar must never damage a conversation.
    catalog: Mutex<Value>,
    catalog_path: PathBuf,
}

fn local(window: &tauri::Window) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("This window has no AG1W capability".into())
    }
}

fn state_path() -> PathBuf {
    if let Some(path) = std::env::var_os("AG1_DESKTOP_STATE") {
        return PathBuf::from(path);
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("AG1W")
}

fn native_default_models(state: &Path) -> Vec<NativeModelSetup> {
    let server = state.join("runtime").join("llama-server.exe");
    let profile = state.join("profiles").join("llamacpp-model-profiles.json");
    let primary = state.join("models").join("Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf");
    let copied_primary = state.join("models").join("35b-copy-complete");
    let primary_projector = state.join("models").join("mmproj-F16.gguf");
    let copied_secondary = state.join("models").join("Qwen3.8-27B-UD-Q4_K_XL.gguf");
    let documents_secondary = std::env::var_os("USERPROFILE").map(|home| {
        PathBuf::from(home)
            .join("Documents")
            .join("models")
            .join("Qwen3.8-27B-UD-Q4_K_XL.gguf")
    });
    // Do not register a partially copied primary model. The copy marker is
    // written only after the source and destination sizes match. AG1W still
    // seals the GGUF with its own SHA-256 before it can start a model.
    if !server.is_file() || !profile.is_file() || !primary.is_file() || !copied_primary.is_file() {
        return Vec::new();
    }

    // Both plans deliberately use one private port. Changing the picker stops
    // the current model before launching the other sealed plan on that port.
    // The 35B model is useful by itself. Its availability must not depend on
    // the optional 27B model also being present on this Windows installation.
    let mut models = vec![NativeModelSetup {
        model_id: "Qwen3.6-35B-A3B-UD-Q4_K_XL".into(),
        server_path: server.clone(),
        model_path: primary,
        profile_path: profile.clone(),
        projector_path: primary_projector.is_file().then_some(primary_projector),
        port: 18084,
        vram_mib: Some(24_576),
        system_prompt: String::new(),
    }];

    let secondary = if copied_secondary.is_file() {
        Some(copied_secondary)
    } else {
        documents_secondary.filter(|path| path.is_file())
    };
    if let Some(model_path) = secondary {
        models.push(NativeModelSetup {
            model_id: "Qwen3.8-27B-UD-Q4_K_XL".into(),
            server_path: server,
            model_path,
            profile_path: profile,
            projector_path: None,
            port: 18084,
            vram_mib: Some(24_576),
            system_prompt: String::new(),
        });
    }
    models
}

fn provision_native_defaults(host: &NativeHost, state: &Path) -> Result<(), String> {
    let defaults = native_default_models(state);
    if defaults.is_empty() {
        return Ok(());
    }

    let models = host.models()?;
    if let Some(configured) = models.as_array().filter(|models| !models.is_empty()) {
        // An old AG1W configuration knows the 35B as text-only. Once its
        // verified projector arrives, replace just that sealed plan before a
        // chat is opened. Later starts read the image capability from the
        // saved catalog and do not hash the large GGUF again.
        let primary = defaults
            .iter()
            .find(|model| model.model_id == "Qwen3.6-35B-A3B-UD-Q4_K_XL");
        let has_text_only_primary = configured.iter().any(|model| {
            model["id"] == "Qwen3.6-35B-A3B-UD-Q4_K_XL"
                && !model["input"]
                    .as_array()
                    .is_some_and(|input| input.iter().any(|kind| kind == "image"))
        });
        if let Some(primary) = primary.filter(|model| model.projector_path.is_some())
            && has_text_only_primary
        {
            tauri::async_runtime::block_on(host.refresh_owned_runtime(primary.clone()))?;
        }
        return Ok(());
    }

    // This hashes and seals local files but never launches llama-server. Doing
    // both together means users do not have to teach a fresh AG1W install about
    // the two native models one dialog at a time.
    tauri::async_runtime::block_on(host.configure_owned_runtimes(defaults))?;
    Ok(())
}

fn default_catalog() -> Value {
    json!({"projects":[],"chats":[],"theme":"dark","themes":[]})
}

fn catalog_path(state: &Path) -> PathBuf {
    state.join("catalog.json")
}

fn read_catalog(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(default_catalog());
    }
    let bytes = fs::read(path).map_err(|_| "Cannot read AG1W sidebar catalog")?;
    if bytes.len() > 1024 * 1024 {
        return Err("AG1W sidebar catalog exceeds 1 MiB".into());
    }
    let catalog: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid AG1W sidebar catalog")?;
    if !catalog.is_object() {
        return Err("Invalid AG1W sidebar catalog".into());
    }
    Ok(catalog)
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn write_catalog(path: &Path, catalog: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(catalog).map_err(|_| "Serialize AG1W sidebar catalog")?;
    if bytes.len() > 1024 * 1024 {
        return Err("AG1W sidebar catalog exceeds 1 MiB".into());
    }
    let parent = path.parent().ok_or("AG1W sidebar catalog has no parent")?;
    let temporary = parent.join("catalog.next.json");
    let write = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("Write AG1W sidebar catalog: {error}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Sync AG1W sidebar catalog: {error}"))
    })();
    if let Err(error) = write {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let source = wide(&temporary);
    let target = wide(path);
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        let error = std::io::Error::last_os_error();
        let _ = fs::remove_file(&temporary);
        return Err(format!("Publish AG1W sidebar catalog: {error}"));
    }
    Ok(())
}

fn catalog_state(state: &Desktop) -> Result<std::sync::MutexGuard<'_, Value>, String> {
    state
        .catalog
        .lock()
        .map_err(|_| "AG1W sidebar state is unavailable".into())
}

fn reply(
    envelope: &CommandEnvelope,
    view: &Value,
    status: &str,
    durable: bool,
    data: Value,
) -> Value {
    let scope = view["snapshot"]["scope"].clone();
    let revision = view["snapshot"]["revision"].clone();
    json!({
        "kind":"reply",
        "id":envelope.id.as_str(),
        "scope":if scope.is_null() { serde_json::to_value(&envelope.scope).unwrap_or(Value::Null) } else { scope },
        "revision":if revision.is_null() { json!(envelope.expected_revision.get().to_string()) } else { revision },
        "durable":durable,
        "status":status,
        "data":data,
    })
}

fn reject(envelope: &CommandEnvelope, view: &Value, detail: impl Into<String>) -> Value {
    reply(
        envelope,
        view,
        "rejected",
        false,
        json!({"error":"Unavailable","detail":detail.into()}),
    )
}

fn current_view(state: &Desktop) -> Result<Value, String> {
    state.host.view()
}

fn check_scope(envelope: &CommandEnvelope, view: &Value) -> Result<(), String> {
    let snapshot = view["snapshot"]
        .as_object()
        .ok_or("Open a local chat before sending a command")?;
    let scope = serde_json::to_value(&envelope.scope).map_err(|_| "Invalid command scope")?;
    if snapshot.get("scope") != Some(&scope)
        || snapshot.get("revision") != Some(&json!(envelope.expected_revision.get().to_string()))
    {
        return Err("The chat changed before this command was admitted. Refresh and retry.".into());
    }
    Ok(())
}

#[tauri::command]
async fn open_general(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    state.host.open_general()
}

#[tauri::command]
async fn open_project(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    path: String,
) -> Result<Value, String> {
    local(&window)?;
    if path.trim().is_empty() || path.len() > 32_768 {
        return Err("Choose a project folder path.".into());
    }
    let view = state.host.open_project(PathBuf::from(path))?;
    let selected = view["snapshot"]["workspace_path"]
        .as_str()
        .ok_or("AG1W did not return the opened project path")?
        .to_owned();
    let mut catalog = catalog_state(&state)?;
    let projects = catalog["projects"]
        .as_array_mut()
        .ok_or("Invalid AG1W project catalog")?;
    let already_saved = projects.iter().any(|project| {
        project
            .as_str()
            .is_some_and(|saved| same_windows_path(saved, &selected))
    });
    if !already_saved {
        // Opening the folder already succeeded. A full recents list should
        // never make that active, verified project disappear behind an error.
        if projects.len() < 128 {
            projects.push(json!(selected));
            write_catalog(&state.catalog_path, &catalog)?;
        }
    }
    Ok(view)
}

fn same_windows_path(left: &str, right: &str) -> bool {
    left.trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .eq_ignore_ascii_case(&right.trim_end_matches(['\\', '/']).replace('/', "\\"))
}

#[tauri::command]
fn desktop_models(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    state.host.models()
}

#[tauri::command]
async fn configure_native_model(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    setup: NativeModelSetup,
) -> Result<Value, String> {
    local(&window)?;
    state.host.configure_owned_runtime(setup).await
}

#[tauri::command]
fn choose_native_runtime_file(
    window: tauri::Window,
    kind: String,
) -> Result<Option<String>, String> {
    local(&window)?;
    let (title, filter) = match kind.as_str() {
        "server" => (
            "Choose llama-server.exe",
            "llama-server executable (*.exe)\0*.exe\0\0",
        ),
        "model" => ("Choose GGUF model", "GGUF model (*.gguf)\0*.gguf\0\0"),
        "projector" => (
            "Choose vision projector GGUF",
            "Vision projector (*.gguf)\0*.gguf\0\0",
        ),
        "profile" => (
            "Choose AG1 model profile",
            "JSON profile (*.json)\0*.json\0\0",
        ),
        _ => return Err("Unknown native runtime file kind".into()),
    };
    choose_file(title, filter)
}

fn choose_file(title: &str, filter: &str) -> Result<Option<String>, String> {
    let mut path = vec![0u16; 32_768];
    let filter = filter.encode_utf16().collect::<Vec<_>>();
    let title = title.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: path.as_mut_ptr(),
        nMaxFile: path.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&mut dialog) } != 0 {
        let length = path
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(path.len());
        return Ok(Some(String::from_utf16_lossy(&path[..length])));
    }
    let code = unsafe { CommDlgExtendedError() };
    if code == 0 {
        Ok(None)
    } else {
        Err(format!("Windows file picker failed ({code})"))
    }
}

fn choose_folder() -> Result<Option<String>, String> {
    let title = "Choose an AG1W project folder"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // Tauri normally owns an STA for the main window. If this command reaches
    // another thread, use the classic picker rather than claiming a modern
    // COM dialog is available with the wrong apartment model.
    let initialized = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    let flags = BIF_RETURNONLYFSDIRS
        | if initialized >= 0 {
            BIF_NEWDIALOGSTYLE
        } else {
            0
        };
    let dialog = BROWSEINFOW {
        lpszTitle: title.as_ptr(),
        ulFlags: flags,
        ..Default::default()
    };
    let item = unsafe { SHBrowseForFolderW(&dialog) };
    if item.is_null() {
        if initialized >= 0 {
            unsafe { CoUninitialize() };
        }
        return Ok(None);
    }
    let mut path = vec![0u16; 32_768];
    let resolved = unsafe { SHGetPathFromIDListW(item, path.as_mut_ptr()) } != 0;
    unsafe { CoTaskMemFree(item.cast()) };
    if initialized >= 0 {
        unsafe { CoUninitialize() };
    }
    if !resolved {
        return Err("Windows folder picker returned a path that AG1W cannot use.".into());
    }
    let length = path
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(path.len());
    Ok(Some(String::from_utf16_lossy(&path[..length])))
}

#[tauri::command]
fn desktop_preferences(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    Ok(state.host.preferences())
}

#[tauri::command]
async fn model_reasoning(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    provider: String,
    id: String,
) -> Result<Value, String> {
    local(&window)?;
    state.host.reasoning_controls(&provider, &id)
}

#[tauri::command]
async fn bridge_command(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    envelope: CommandEnvelope,
) -> Result<Value, String> {
    local(&window)?;
    let before = current_view(&state)?;
    let result = match &envelope.command {
        Command::Prompt {
            input: PromptInput::Inline { text },
        } => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                match state.host.begin_prompt(text) {
                    Ok(()) => {
                        // Admit first, then release the Tauri command. Holding
                        // this invoke open hid streamed tokens until completion.
                        let host = state.host.clone();
                        let request = text.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = host.run_started_prompt(request).await;
                        });
                        reply(
                            &envelope,
                            &current_view(&state)?,
                            "accepted",
                            false,
                            json!({"kind":"accepted","operation":{"operation":"local-generation","status":"running"}}),
                        )
                    }
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Prompt {
            input: PromptInput::MessageArtifact { artifact },
        } => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                match state.host.begin_uploaded_prompt(artifact.as_str()) {
                    Ok(message) => {
                        let host = state.host.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = host.run_started_message(message).await;
                        });
                        reply(
                            &envelope,
                            &current_view(&state)?,
                            "accepted",
                            false,
                            json!({"kind":"accepted","operation":{"operation":"local-generation","status":"running"}}),
                        )
                    }
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Prompt { .. } => reject(
            &envelope,
            &before,
            "AG1W accepts inline text or pasted message attachments.",
        ),
        Command::Artifacts { action, args } => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let data = if action == "list" {
                    state.host.artifact_catalog(args)
                } else {
                    state.host.upload_attachment(action, args)
                };
                match data {
                    Ok(data) => reply(&envelope, &current_view(&state)?, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::ArtifactRead {
            artifact,
            offset,
            limit,
        } => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                match state
                    .host
                    .read_artifact(artifact.as_str(), offset.get(), *limit)
                {
                    Ok(data) => reply(&envelope, &before, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Model { action, args } if action == "select" => {
            let provider = args["provider"].as_str().ok_or("Missing model provider")?;
            let id = args["model_id"].as_str().ok_or("Missing model ID")?;
            match state.host.select_model(provider, id) {
                Ok(()) => reply(
                    &envelope,
                    &current_view(&state)?,
                    "accepted",
                    true,
                    Value::Null,
                ),
                Err(error) => reject(&envelope, &before, error),
            }
        }
        Command::Model { action, args } if action == "context" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let tokens = args["tokens"]
                    .as_u64()
                    .ok_or("Missing working context token limit")?;
                match state.host.set_context_limit(tokens) {
                    Ok(()) => reply(
                        &envelope,
                        &current_view(&state)?,
                        "accepted",
                        true,
                        json!({"tokens":tokens,"scope":"current-chat"}),
                    ),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Task { action, args } => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                match state.host.task_action(action, args) {
                    Ok(data) => reply(&envelope, &current_view(&state)?, "accepted", true, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Plan { action, args } if action == "view" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                match state.host.plan_snapshot() {
                    Ok(data) => reply(&envelope, &before, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Plan { action, args } if action == "draft" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let input = &args["input"];
                if input["kind"] != "inline" {
                    reject(
                        &envelope,
                        &before,
                        "Native plan mode currently accepts text objectives only",
                    )
                } else {
                    let goal = input["text"]
                        .as_str()
                        .ok_or("Missing plan objective")?
                        .to_owned();
                    let previous = if args["previous"].is_null() {
                        None
                    } else {
                        Some(PlanRef {
                            plan_id: args["previous"]["planId"]
                                .as_str()
                                .filter(|value| !value.is_empty())
                                .ok_or("Invalid previous plan")?
                                .into(),
                            revision: args["previous"]["revision"]
                                .as_u64()
                                .filter(|value| *value > 0)
                                .ok_or("Invalid previous plan revision")?,
                        })
                    };
                    let feedback = args["feedback"].as_str().map(str::to_owned);
                    match state.host.begin_plan_draft(&goal) {
                        Ok(command) => {
                            let host = state.host.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = host
                                    .run_started_plan_draft(command, goal, previous, feedback)
                                    .await;
                            });
                            reply(
                                &envelope,
                                &current_view(&state)?,
                                "accepted",
                                false,
                                json!({"kind":"accepted","operation":{"operation":"local-plan-draft","status":"running"}}),
                            )
                        }
                        Err(error) => reject(&envelope, &before, error),
                    }
                }
            }
        }
        Command::Plan { action, args } => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                match state.host.change_plan(action, args) {
                    Ok(data) => reply(&envelope, &current_view(&state)?, "accepted", true, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Runtime { action, args } if action == "load_model" => {
            let provider = args["provider"].as_str().ok_or("Missing model provider")?;
            let id = args["model_id"].as_str().ok_or("Missing model ID")?;
            match state.host.begin_model_load(provider, id) {
                Ok(()) => {
                    // Loading a 22 GiB model can take minutes. Acknowledge the
                    // click now; `bridge_view` exposes the verified live phase.
                    let host = state.host.clone();
                    let model = id.to_owned();
                    tauri::async_runtime::spawn(async move {
                        let _ = host.run_started_model_load(model).await;
                    });
                    reply(
                        &envelope,
                        &current_view(&state)?,
                        "accepted",
                        false,
                        json!({"kind":"accepted","operation":{"operation":"local-model-load","status":"running"}}),
                    )
                }
                Err(error) => reject(&envelope, &before, error),
            }
        }
        Command::Runtime { action, args } if action == "approval_policy" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let mode = args["mode"].as_str().ok_or("Missing approval mode")?;
                if !matches!(mode, "interactive" | "autonomous" | "disabled") {
                    reject(&envelope, &before, "Unknown approval mode")
                } else {
                    // The shared composer sends this immediately before a
                    // prompt. AG1W keeps the choice in this chat only, then
                    // turns it into bounded project-write access at execution.
                    match state.host.set_approval_policy(mode) {
                        Ok(()) => reply(
                            &envelope,
                            &current_view(&state)?,
                            "accepted",
                            false,
                            json!({"mode":mode,"scope":"current-chat"}),
                        ),
                        Err(error) => reject(&envelope, &before, error),
                    }
                }
            }
        }
        Command::Runtime { action, args } if action == "folder_access" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                match state.host.change_folder_access(args) {
                    Ok(data) => reply(&envelope, &current_view(&state)?, "accepted", true, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Runtime { action, args } if action == "website_icon" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                // Icon fetching is bounded but can wait on a remote TLS server.
                // Keep that wait off Tauri's command executor so streamed chat
                // updates and the rest of the desktop remain responsive.
                let host = state.host.clone();
                let args = args.clone();
                match tauri::async_runtime::spawn_blocking(move || host.website_icon(&args)).await {
                    Ok(Ok(data)) => reply(&envelope, &before, "accepted", false, data),
                    Ok(Err(error)) => reject(&envelope, &before, error),
                    Err(_) => reject(
                        &envelope,
                        &before,
                        "AG1W source-icon worker stopped unexpectedly",
                    ),
                }
            }
        }
        Command::Runtime { action, args } if action == "mcp_manager" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                // Reviewing a local executable includes a bounded file hash and
                // can take noticeable time on a slow disk. Keep that work off
                // Tauri's command executor so the chat remains responsive.
                let host = state.host.clone();
                let args = args.clone();
                match tauri::async_runtime::spawn_blocking(move || host.mcp_action(&args)).await {
                    Ok(Ok(data)) => {
                        reply(&envelope, &current_view(&state)?, "accepted", false, data)
                    }
                    Ok(Err(error)) => reject(&envelope, &before, error),
                    Err(_) => reject(&envelope, &before, "AG1W MCP worker stopped unexpectedly"),
                }
            }
        }
        Command::Runtime { action, .. } if action == "status" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                match state.host.runtime_status() {
                    Ok(data) => reply(&envelope, &before, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Files { action, args } => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let data = match action.as_str() {
                    "list" => state.host.list_files(),
                    "diff" if args["paged"] == true => state.host.workspace_diff(args),
                    "changes" => state.host.turn_changes(args),
                    "change_read" => state.host.read_turn_change(args),
                    "change_undo_status" => state.host.change_undo_status(args),
                    "change_undo" => state.host.change_undo(args, false),
                    "change_undo_recover" => state.host.change_undo(args, true),
                    "read" => args["path"]
                        .as_str()
                        .ok_or_else(|| "Missing file path".to_owned())
                        .and_then(|path| state.host.read_file(path)),
                    "save" => {
                        let path = args["path"].as_str().ok_or("Missing file path")?;
                        let text = args["text"].as_str().ok_or("Missing file text")?;
                        let revision = args["revision"].as_str().ok_or("Missing file revision")?;
                        state.host.save_file(path, text, revision)
                    }
                    _ => Err(state.host.unsupported("That project-file action")),
                };
                match data {
                    Ok(data) => reply(&envelope, &current_view(&state)?, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, args } if action == "compact" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let focus = args["focus"].as_str().map(str::to_owned);
                match state.host.begin_context_compaction(focus.as_deref()) {
                    Ok(command) => {
                        // Compaction can take a normal local-model request.
                        // Acknowledge it now so the renderer can show its live
                        // state and offer the same Stop control as generation.
                        let host = state.host.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = host.run_started_context_compaction(command, focus).await;
                        });
                        reply(
                            &envelope,
                            &current_view(&state)?,
                            "accepted",
                            false,
                            json!({"kind":"accepted","operation":{"operation":"local-context-compaction","status":"running"}}),
                        )
                    }
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, args } if action == "messages" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let before_message = args["before"]
                    .as_u64()
                    .ok_or("Missing message page position")?
                    .try_into()
                    .map_err(|_| "Message page position is too large")?;
                match state.host.session_messages(before_message) {
                    Ok(data) => reply(&envelope, &before, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, args } if action == "entry" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let id = args["id"]
                    .as_str()
                    .ok_or("Missing durable journal entry ID")?;
                let offset = args["offset"]
                    .as_str()
                    .ok_or("Missing durable journal entry offset")?
                    .parse::<usize>()
                    .map_err(|_| "Invalid durable journal entry offset")?;
                match state.host.session_entry(id, offset) {
                    Ok(data) => reply(&envelope, &before, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, args } if action == "catalog" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                match state.host.catalog_sessions(offset, args["current"] == true) {
                    Ok(data) => reply(&envelope, &before, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, args } if action == "history" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                match state.host.session_history(offset) {
                    Ok(data) => reply(&envelope, &before, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, args } if action == "export" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let path = args["path"]
                    .as_str()
                    .ok_or("Missing HTML export filename")?;
                match state.host.export_session_html(path) {
                    Ok(data) => reply(&envelope, &current_view(&state)?, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, args } if action == "user_text" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let id = args["id"].as_str().ok_or("Missing user journal entry ID")?;
                let offset = args["offset"]
                    .as_str()
                    .ok_or("Missing user journal entry offset")?
                    .parse::<usize>()
                    .map_err(|_| "Invalid user journal entry offset")?;
                match state.host.session_user_text(id, offset) {
                    Ok(data) => reply(&envelope, &before, "accepted", false, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, args } if action == "fork" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let entry = args["entry"]
                    .as_str()
                    .ok_or("Missing user journal entry ID")?;
                match state.host.fork_before_user_message(entry) {
                    Ok(data) => reply(&envelope, &current_view(&state)?, "accepted", true, data),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, args } if action == "tree" => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                let target = args["target"]
                    .as_str()
                    .ok_or("Missing local branch target")?;
                let summarize = args["summarize"].as_bool().unwrap_or(false);
                match state.host.begin_history_selection(target, summarize).await {
                    Ok(data) => {
                        // A summary may take a full local-model turn. Return its
                        // operation now so the shared renderer can poll the same
                        // terminal receipt protocol it already uses for history.
                        if data["pending"] == true {
                            let host = state.host.clone();
                            let target = target.to_owned();
                            let operation = data["operation"]
                                .as_str()
                                .ok_or("Native branch summary has no operation ID")?
                                .to_owned();
                            tauri::async_runtime::spawn(async move {
                                let _ = host.run_started_history_selection(target, operation).await;
                            });
                        }
                        reply(&envelope, &current_view(&state)?, "accepted", true, data)
                    }
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Session { action, .. } if action == "new" => match state.host.new_chat() {
            Ok(view) => reply(
                &envelope,
                &view,
                "accepted",
                true,
                json!({"kind":"settled","operation":{"operation":"navigation","status":"completed"}}),
            ),
            Err(error) => reject(&envelope, &before, error),
        },
        Command::CommandResult { command } => reply(
            &envelope,
            &before,
            "accepted",
            false,
            json!({"kind":"settled","operation":{"operation":command.as_str(),"status":"completed"}}),
        ),
        Command::Cancel { .. } => {
            if let Err(error) = check_scope(&envelope, &before) {
                reject(&envelope, &before, error)
            } else {
                match state.host.cancel() {
                    Ok(()) => reply(
                        &envelope,
                        &current_view(&state)?,
                        "accepted",
                        false,
                        json!({"kind":"accepted","operation":{"operation":"local-generation","status":"cancelling"}}),
                    ),
                    Err(error) => reject(&envelope, &before, error),
                }
            }
        }
        Command::Model { .. } => reject(
            &envelope,
            &before,
            "Only local model selection is available in this Windows-native slice",
        ),
        _ => reject(
            &envelope,
            &before,
            state.host.unsupported("That AG1 desktop capability"),
        ),
    };
    Ok(result)
}

#[tauri::command]
async fn bridge_view(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    current_view(&state)
}

#[tauri::command]
async fn bridge_live(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    after: u64,
) -> Result<Value, String> {
    local(&window)?;
    let mut view = current_view(&state)?;
    let mut revision = view["details"]["activity"]["live_revision"]
        .as_u64()
        .unwrap_or(0);
    if revision <= after {
        // This is a tiny long-poll, not a busy loop. The React client can get
        // a new token block promptly without hammering the Tauri bridge.
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        view = current_view(&state)?;
        revision = view["details"]["activity"]["live_revision"]
            .as_u64()
            .unwrap_or(0);
    }
    Ok(json!({
        "scope":view["snapshot"]["scope"],
        "revision":revision,
        "observations":view["details"]["activity"]["observations"],
    }))
}

#[tauri::command]
fn catalog(window: tauri::Window, state: tauri::State<'_, Arc<Desktop>>) -> Result<Value, String> {
    local(&window)?;
    Ok(catalog_state(&state)?.clone())
}

#[tauri::command]
fn save_chat(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    chat: Value,
) -> Result<(), String> {
    local(&window)?;
    if !chat.is_object() || chat.to_string().len() > 16 * 1024 {
        return Err("Invalid chat sidebar entry".into());
    }
    let session = chat["session"].as_str().ok_or("Chat session is missing")?;
    let workspace = chat["workspace"]
        .as_str()
        .ok_or("Chat workspace is missing")?;
    let mut catalog = catalog_state(&state)?;
    let chats = catalog["chats"]
        .as_array_mut()
        .ok_or("Invalid chat catalog")?;
    if let Some(existing) = chats
        .iter_mut()
        .find(|entry| entry["session"] == session && entry["workspace"] == workspace)
    {
        *existing = chat;
    } else if chats.len() < 512 {
        chats.push(chat);
    } else {
        return Err("Chat sidebar is full".into());
    }
    write_catalog(&state.catalog_path, &catalog)
}

#[tauri::command]
fn save_preferences(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    theme: String,
) -> Result<(), String> {
    local(&window)?;
    let mut catalog = catalog_state(&state)?;
    if !themes::is_builtin(&theme)
        && !catalog["themes"]
            .as_array()
            .is_some_and(|themes| themes.iter().any(|entry| entry["id"] == theme))
    {
        return Err("Theme is not installed".into());
    }
    catalog["theme"] = json!(theme);
    write_catalog(&state.catalog_path, &catalog)
}

#[tauri::command]
fn import_theme(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    plugin: Value,
) -> Result<(), String> {
    local(&window)?;
    let plugin = serde_json::to_value(themes::validate(plugin)?).map_err(|_| "Serialize theme")?;
    let mut catalog = catalog_state(&state)?;
    let themes = catalog["themes"]
        .as_array_mut()
        .ok_or("Invalid theme catalog")?;
    if themes.iter().any(|entry| entry["id"] == plugin["id"]) || themes.len() >= 64 {
        return Err("Theme already exists or the theme library is full".into());
    }
    themes.push(plugin);
    write_catalog(&state.catalog_path, &catalog)
}

#[tauri::command]
fn remove_theme(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    id: String,
) -> Result<(), String> {
    local(&window)?;
    let mut catalog = catalog_state(&state)?;
    let themes = catalog["themes"]
        .as_array_mut()
        .ok_or("Invalid theme catalog")?;
    themes.retain(|entry| entry["id"] != id);
    if catalog["theme"] == id {
        catalog["theme"] = json!("dark");
    }
    write_catalog(&state.catalog_path, &catalog)
}

#[tauri::command]
async fn choose_chat_folder(
    window: tauri::Window,
    _state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Option<String>, String> {
    local(&window)?;
    choose_folder()
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

    // Schemas and the catalog are bundled data. They never log in, make a
    // request, or expose an account to the renderer.
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
        state.github.shared(state.host.state_path())?.cancel();
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

        // One UI transition at a time keeps previews tied to a stable chat
        // scope and prevents a double-click from publishing two operations.
        let _transition = state.transition.lock().await;
        let scope = state.host.github_scope()?;
        let state_path = state.host.state_path().to_owned();
        let service = state.github.shared(&state_path)?;
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

    if matches!(action.as_str(), "local" | "checkout" | "push" | "clone") {
        return Err("Local Git checkout and push are not ported to AG1W yet.".into());
    }

    let state_path = state.host.state_path().to_owned();
    let github = state.github.clone();
    tauri::async_runtime::spawn_blocking(move || github.request(&state_path, &action, args, None))
        .await
        .map_err(|_| "GitHub action interrupted".to_string())?
}

#[tauri::command]
async fn conversation_statuses(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<Value, String> {
    local(&window)?;
    state.host.conversation_statuses()
}

#[tauri::command]
async fn select_conversation(
    window: tauri::Window,
    state: tauri::State<'_, Arc<Desktop>>,
    session: Option<String>,
) -> Result<Value, String> {
    local(&window)?;
    match session {
        Some(session) => state.host.open_session(&session),
        None => state.host.open_general(),
    }
}

#[tauri::command]
fn cutover_status(
    window: tauri::Window,
    _state: tauri::State<'_, Arc<Desktop>>,
    path: String,
) -> Result<Value, String> {
    local(&window)?;
    // The Windows host owns a separate authority registry. Reading it here
    // lets the recovery UI describe a real interrupted handoff instead of
    // silently treating every workspace as if it had no state.
    let record = ag1_platform::cutover::Registry::for_user()
        .and_then(|registry| registry.inspect(std::path::Path::new(&path)))
        .map_err(|error| format!("{error:?}"))?;
    serde_json::to_value(record).map_err(|error| error.to_string())
}

#[tauri::command]
async fn record_rendering_sample(
    window: tauri::Window,
    _state: tauri::State<'_, Arc<Desktop>>,
    _sample: Value,
) -> Result<(), String> {
    local(&window)
}

#[tauri::command]
async fn quit(
    window: tauri::Window,
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Desktop>>,
) -> Result<(), String> {
    local(&window)?;
    state.github.shutdown();
    state.github.wait_stopped()?;
    state.host.shutdown().await?;
    app.exit(0);
    Ok(())
}

pub fn run() {
    let state = state_path();
    let host = NativeHost::open(state.clone()).expect("AG1W private desktop state unavailable");
    if let Err(error) = provision_native_defaults(&host, &state) {
        eprintln!("AG1W could not provision its native model catalog: {error}");
    }
    let catalog_path = catalog_path(&state);
    let loaded_catalog = read_catalog(&catalog_path).expect("AG1W sidebar catalog unavailable");
    let desktop = Arc::new(Desktop {
        host,
        github: Arc::new(ag1_platform::github::GitHub::default()),
        transition: tokio::sync::Mutex::new(()),
        catalog: Mutex::new(loaded_catalog),
        catalog_path,
    });
    tauri::Builder::default()
        .manage(desktop)
        .invoke_handler(tauri::generate_handler![
            open_project,
            open_general,
            record_rendering_sample,
            select_conversation,
            conversation_statuses,
            desktop_models,
            configure_native_model,
            choose_native_runtime_file,
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
            remove_theme,
            quit
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Do not leave an owned model process behind when the title-bar
                // close button is used. The async task exits only after the
                // Job-owned runtime reports its shutdown result.
                api.prevent_close();
                let host = window.state::<Arc<Desktop>>().host.clone();
                let github = window.state::<Arc<Desktop>>().github.clone();
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    github.shutdown();
                    let _ = github.wait_stopped();
                    let _ = host.shutdown().await;
                    app.exit(0);
                });
            }
        })
        .run(tauri::generate_context!())
        .expect("AG1W desktop launch failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_catalog_survives_a_new_desktop_process() {
        let directory = tempfile::tempdir().unwrap();
        let path = catalog_path(directory.path());
        let catalog = json!({
            "projects": [],
            "chats": [{
                "session": "win-session-1-1",
                "workspace": "general-chat",
                "title": "Durable local chat"
            }],
            "theme": "dark",
            "themes": []
        });

        write_catalog(&path, &catalog).unwrap();
        assert_eq!(read_catalog(&path).unwrap(), catalog);
    }
}
