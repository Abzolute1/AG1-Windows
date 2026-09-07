#[cfg(feature = "visual-test")]
fn main() {
    use ag1_platform::{
        projects::{Command, Launch, ReadyHttp, Runtime, Toolchains},
        resource_fs::ScanControl,
        workspace::WorkspaceService,
    };
    use javascriptcore::ValueExt;
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex, atomic::AtomicBool},
        time::{Duration, Instant},
    };
    use tauri::Manager;
    use webkit2gtk::WebViewExt;
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(1);
    }));
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let output = PathBuf::from(std::env::args().nth(1).expect("private evidence output"));
    std::fs::create_dir_all(&output).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let state = temp.path().join("state");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&state).unwrap();
    let ws = WorkspaceService::new().unwrap();
    let mut runtime = Runtime::load(
        ws.register(&root).unwrap(),
        ws.register(&state).unwrap(),
        &repo.join("product/target/debug/ag1-process-host"),
        Toolchains {
            node: None,
            npm: None,
            cargo: None,
            rustc: None,
            godot: None,
            mounts: vec![],
        },
    )
    .unwrap();
    runtime.workspace.create_checked("index.html",b"<!doctype html><title>Interactive project preview</title><h1>Interactive project preview</h1><button id=increment>0</button><script>document.querySelector('button').onclick=e=>e.target.textContent=String(Number(e.target.textContent)+1)</script>",false,||Ok(())).unwrap();
    runtime
        .workspace
        .create_checked("ready", b"p11-private-preview", false, || Ok(()))
        .unwrap();
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(60),
    };
    runtime
        .open_private_display(&repo.join(".local/p11/xvfb/usr/bin/Xvfb"), &control)
        .unwrap();
    // Before GTK or Tauri threads start. P03's only background reader performs
    // file IO and never reads the environment; this is a test-only executable.
    for (k, v) in runtime.private_display_environment().unwrap() {
        unsafe {
            std::env::set_var(k, v);
        }
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let target = runtime.project.targets[0].id.clone();
    runtime
        .start(
            &target,
            "run",
            Launch {
                commands: vec![Command {
                    executable: "/usr/bin/python3".into(),
                    argv: vec![
                        "-m".into(),
                        "http.server".into(),
                        address.port().to_string(),
                        "--bind".into(),
                        "127.0.0.1".into(),
                    ],
                    cwd: "".into(),
                }],
                readiness: Some(ReadyHttp {
                    address,
                    path: "/ready".into(),
                    expected_body: "p11-private-preview".into(),
                }),
                timeout_ms: 60000,
            },
            &control,
        )
        .unwrap();
    while runtime.poll(&control).unwrap().lifecycle != ag1_core::projects::Lifecycle::Ready {
        std::thread::sleep(Duration::from_millis(10));
    }
    let runtime = Arc::new(Mutex::new(runtime));
    let owned = runtime.clone();
    let app=tauri::Builder::default().invoke_handler(tauri::generate_handler![fixture_privileged]).setup(move|app|{
        app.get_webview_window("main").unwrap().hide().unwrap();
        let preview=ag1_desktop::project_preview::ProjectPreview::open(app.handle(),&owned.lock().unwrap()).unwrap();preview.focus(&owned.lock().unwrap()).unwrap();
        let handle=app.handle().clone();
        std::thread::spawn(move||{
            std::thread::sleep(Duration::from_secs(2));
            let window=preview.window().clone();
            let(tx,rx)=std::sync::mpsc::sync_channel(1);
            window.with_webview(move|view|view.inner().evaluate_javascript("document.querySelector('#increment').click();window.__denied=false;if(window.__TAURI_INTERNALS__)window.__TAURI_INTERNALS__.invoke('fixture_privileged').then(()=>window.__denied='ALLOWED',()=>window.__denied=true);else window.__denied=true;window.open('https://example.invalid/');true",None,None,None::<&gio::Cancellable>,move|r|{tx.send(r.is_ok()).unwrap();})).unwrap();assert!(rx.recv_timeout(Duration::from_secs(10)).unwrap());
            std::thread::sleep(Duration::from_millis(300));let(tx,rx)=std::sync::mpsc::sync_channel(1);
            window.with_webview(move|view|view.inner().evaluate_javascript("({title:document.title,count:document.querySelector('button').textContent,denied:window.__denied,url:location.href,node:typeof process})",None,None,None::<&gio::Cancellable>,move|r|{tx.send(r.unwrap().to_json(0).unwrap().to_string()).unwrap();})).unwrap();let value=rx.recv_timeout(Duration::from_secs(10)).unwrap();let result:serde_json::Value=serde_json::from_str(&value).unwrap();assert_eq!(result["count"],"1");assert_eq!(result["denied"],true);assert_eq!(result["node"],"undefined");assert_eq!(handle.webview_windows().len(),2);
            let(tx,rx)=std::sync::mpsc::sync_channel(1);let capture=output.join("interactive-webkit.png");window.with_webview(move|view|view.inner().snapshot(webkit2gtk::SnapshotRegion::Visible,webkit2gtk::SnapshotOptions::NONE,None::<&gio::Cancellable>,move|r|{r.unwrap().write_to_png(&mut std::fs::File::create(capture).unwrap()).unwrap();tx.send(()).unwrap();})).unwrap();rx.recv_timeout(Duration::from_secs(10)).unwrap();
            window.eval("location.href='https://example.invalid/escape'").unwrap();std::thread::sleep(Duration::from_millis(200));assert_eq!(window.url().unwrap().origin().ascii_serialization(),format!("http://{address}"));
            let mut preview=preview;preview.close().unwrap();let stopped=owned.lock().unwrap().stop().unwrap().unwrap();
            std::fs::write(output.join("interactive-webkit.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","observation":result,"blocked_navigation":true,"closed_preview":true,"stopped":stopped})).unwrap()).unwrap();handle.exit(0);
        });Ok(())
    }).build(tauri::generate_context!("project-fixture.conf.json")).unwrap();
    app.run(|_, _| {});
    runtime.lock().unwrap().close_private_display().unwrap();
    drop(runtime);
    drop(temp);
}
#[cfg(feature = "visual-test")]
#[tauri::command]
fn fixture_privileged(window: tauri::Window) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("This window has no agent capability".into())
    }
}
#[cfg(not(feature = "visual-test"))]
fn main() {
    panic!("Build this probe with --features visual-test");
}
