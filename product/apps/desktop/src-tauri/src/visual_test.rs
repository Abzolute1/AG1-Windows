//! Explicit test build only. No broker is present in production builds.
use javascriptcore::ValueExt;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};
use tauri::Manager;
use webkit2gtk::WebViewExt;
pub fn setup(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("AG1_FIXTURE_PROVIDER").as_deref() != Ok("1") {
        return Err("Visual broker requires fixture provider".into());
    }
    let path = PathBuf::from(std::env::var("AG1_VISUAL_SOCKET")?);
    if path.exists() {
        return Err("Visual socket already exists".into());
    }
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    let window = app
        .get_webview_window("main")
        .ok_or("main webview missing")?;
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { break };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() || line.len() > 65536 {
                continue;
            }
            let response = (|| -> Result<Value, String> {
                let request: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
                let (tx, rx) = mpsc::sync_channel(1);
                if let Some(script) = request["script"].as_str() {
                    let script = script.to_owned();
                    window
                        .with_webview(move |view| {
                            view.inner().evaluate_javascript(
                                &script,
                                None,
                                None,
                                None::<&gio::Cancellable>,
                                move |result| {
                                    let result =
                                        result.map_err(|e| e.to_string()).and_then(|value| {
                                            serde_json::from_str(
                                                value.to_json(0).as_deref().unwrap_or("null"),
                                            )
                                            .map_err(|e| e.to_string())
                                        });
                                    let _ = tx.send(result);
                                },
                            );
                        })
                        .map_err(|e| e.to_string())?;
                } else if let Some(name) = request["screenshot"].as_str() {
                    if !name.ends_with(".png") || name.contains('/') || name.contains("..") {
                        return Err("Invalid screenshot name".into());
                    }
                    let output = path.parent().unwrap().join(name);
                    window
                        .with_webview(move |view| {
                            view.inner().snapshot(
                                webkit2gtk::SnapshotRegion::Visible,
                                webkit2gtk::SnapshotOptions::NONE,
                                None::<&gio::Cancellable>,
                                move |result| {
                                    let result =
                                        result.map_err(|e| e.to_string()).and_then(|surface| {
                                            let mut file = fs::File::create(output)
                                                .map_err(|e| e.to_string())?;
                                            surface
                                                .write_to_png(&mut file)
                                                .map_err(|e| e.to_string())?;
                                            Ok(json!(true))
                                        });
                                    let _ = tx.send(result);
                                },
                            );
                        })
                        .map_err(|e| e.to_string())?;
                } else {
                    return Err("Unknown visual request".into());
                }
                rx.recv_timeout(Duration::from_secs(15))
                    .map_err(|_| "Native visual operation timed out".to_owned())?
            })();
            let payload = match response {
                Ok(value) => json!({"ok":value}),
                Err(error) => json!({"error":error}),
            };
            let _ = writeln!(stream, "{payload}");
        }
    });
    Ok(())
}
