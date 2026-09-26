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
    let fixture = std::env::var("AG1_FIXTURE_PROVIDER").as_deref() == Ok("1");
    if let Some(workspace) = std::env::var_os("AG1_RELEASE_WORKSPACE") {
        if fixture || std::env::var_os("AG1_CUTOVER_WORKSPACE").is_some() {
            return Err("Release visual mode cannot combine fixture or migrated authority".into());
        }
        let workspace = std::fs::canonicalize(workspace)?;
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .join(".local/p14-long-horizon")
            .canonicalize()?;
        if !workspace.starts_with(&base) {
            return Err("Release visual mode requires a disposable P14 workspace".into());
        }
        let configuration = PathBuf::from(
            std::env::var_os("AG1_NATIVE_CONFIG")
                .ok_or("Release visual mode requires explicit native configuration")?,
        )
        .canonicalize()?;
        if !configuration.starts_with(workspace.parent().ok_or("Workspace parent")?) {
            return Err(
                "Release visual configuration must belong to the disposable workload".into(),
            );
        }
        let bytes = fs::read(&configuration)?;
        if bytes.len() > 1024 * 1024 {
            return Err("Native configuration too large".into());
        }
        let config: ag1_platform::client::Config = serde_json::from_slice(&bytes)?;
        if config.fixture || config.runtime.is_none() {
            return Err("Release visual mode requires ordinary preserved-runtime admission".into());
        }
        let record = ag1_platform::cutover::Registry::for_user()
            .and_then(|registry| registry.inspect(&workspace))
            .map_err(|_| "Cannot inspect disposable native authority")?
            .ok_or("Disposable native authority is absent")?;
        if !record.isolated
            || !record.quiescent
            || record.owner != ag1_platform::cutover::Owner::Native
            || record.stage != ag1_platform::cutover::Stage::Active
            || record.transfer.is_some()
            || record.rollback.is_some()
            || record
                .state
                .as_ref()
                .is_none_or(|state| !state.starts_with(workspace.parent().unwrap()))
        {
            return Err("Release visual mode requires quiescent isolated native state".into());
        }
        // The subsequent ordinary Host::open still obtains the workspace/runtime
        // leases and verifies the full preserved capture. This is test-broker
        // admission only; no fixture owner or alternative runtime is created.
    } else if let Some(workspace) = std::env::var_os("AG1_CUTOVER_WORKSPACE") {
        if fixture || std::env::var_os("AG1_NATIVE_CONFIG").is_some() {
            return Err(
                "Cutover visual mode requires the recorded production configuration".into(),
            );
        }
        let audit = ag1_platform::cutover::Registry::for_user()
            .and_then(|registry| registry.audit_cutover(std::path::Path::new(&workspace)))
            .map_err(|_| "Cutover visual mode requires an audited quiescent native workspace")?;
        if audit["fixture"] != false || audit["status"] != "production_authority_verified" {
            return Err("Fixture authority cannot enable production visual measurement".into());
        }
    } else if !fixture {
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
    let desktop = app
        .state::<std::sync::Arc<ag1_desktop::Desktop>>()
        .inner()
        .clone();
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
                if request["native_view"] == true {
                    // Observe the native host independently of the renderer's
                    // JavaScript thread. This grants no mutation capability.
                    return tauri::async_runtime::block_on(async {
                        let host = desktop.native.host().await?;
                        let view = host.settled_view().await.map_err(|e| format!("{e:?}"))?;
                        let configuration = host
                            .registry
                            .selected()
                            .map_err(|e| format!("{e:?}"))?
                            .services
                            .runtime()
                            .configuration()
                            .map_err(|e| format!("{e:?}"))?;
                        let tools: Vec<&str> = configuration
                            .tools
                            .iter()
                            .filter_map(|tool| tool["name"].as_str())
                            .collect();
                        Ok(json!({"view":view,"tools":tools}))
                    });
                }
                if let Some(size) = request["size"].as_array() {
                    let width = size
                        .first()
                        .and_then(Value::as_u64)
                        .filter(|n| (640..=3840).contains(n))
                        .ok_or("Visual width")?;
                    let height = size
                        .get(1)
                        .and_then(Value::as_u64)
                        .filter(|n| (500..=2160).contains(n))
                        .ok_or("Visual height")?;
                    window
                        .set_size(tauri::PhysicalSize::new(width as u32, height as u32))
                        .map_err(|e| e.to_string())?;
                    return Ok(json!({"resized":true}));
                }
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

/// Test-only TLS composition is loaded before native OAuth/manager creation.
/// The manifest lives beside the authenticated visual socket in an owned run.
pub fn configure_mcp_tls()->Result<(),Box<dyn std::error::Error>> {
    let Some(socket)=std::env::var_os("AG1_VISUAL_SOCKET") else {return Ok(())};
    let socket=PathBuf::from(socket);
    let parent=socket.parent().ok_or("Visual socket parent")?.canonicalize()?;
    let base=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../.local/p14-live-fixes").canonicalize()?;
    let path=parent.join("owned-mcp-tls.json");
    if !path.exists() {return Ok(());}
    if std::env::var("AG1_FIXTURE_PROVIDER").as_deref()!=Ok("1") || !parent.starts_with(&base) || !parent.parent().and_then(|p|p.file_name()).is_some_and(|p|p.to_string_lossy().starts_with("native-candidate-mcp-c2-")) {
        return Err("Owned MCP TLS requires an isolated C2 fixture run".into());
    }
    use std::os::unix::fs::{MetadataExt,OpenOptionsExt};
    let mut file=std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&path)?;
    let meta=file.metadata()?;
    if !meta.is_file() || meta.len()>100000 || meta.permissions().mode()&0o077!=0 || meta.uid()!=unsafe{libc::geteuid()} {return Err("Owned MCP TLS manifest identity".into());}
    let mut bytes=Vec::new();std::io::Read::read_to_end(&mut std::io::Read::take(&mut file,100001),&mut bytes)?;
    if bytes.len()>100000 {return Err("Owned MCP TLS manifest quota".into());}
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Manifest {origin:String,certificate_pem:String}
    let manifest:Manifest=serde_json::from_slice(&bytes)?;
    ag1_platform::mcp::fixtures::TlsPeer::install(&manifest.origin,manifest.certificate_pem.as_bytes()).map_err(|_|"Owned MCP TLS policy invalid")?;
    Ok(())
}
