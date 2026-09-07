//! Generated-app fixture: its own Tauri runtime has no AG1 commands or state.
//! Use --features visual-test for private WebKit evidence, never normal desktop.
#[cfg(feature = "visual-test")]
fn main() {
    // Deliberate crash evidence needs its exit and retained PNG, not a host core dump.
    unsafe {
        libc::setrlimit(
            libc::RLIMIT_CORE,
            &libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            },
        );
    }
    use javascriptcore::ValueExt;
    use tauri::Manager;
    use webkit2gtk::WebViewExt;
    let crash = std::env::args().any(|a| a == "--crash");
    tauri::Builder::default().setup(move|app|{
        let window=app.get_webview_window("main").unwrap();
        let handle=app.handle().clone();
        std::thread::spawn(move||{
            std::thread::sleep(std::time::Duration::from_secs(2));
            let handle2=handle.clone();
            window.with_webview(move|view|{
                let webview=view.inner().clone();
                view.inner().evaluate_javascript("document.body.innerHTML='<h1>Native Tauri fixture</h1><button id=increment>0</button>';document.querySelector('#increment').onclick=e=>e.target.textContent=String(Number(e.target.textContent)+1);document.querySelector('#increment').click();({title:document.querySelector('h1').textContent,count:document.querySelector('button').textContent,node:typeof process})",None,None,None::<&gio::Cancellable>,move|result|{
                    let result=result.map(|v|v.to_json(0).unwrap().to_string()).unwrap();println!("P11_NATIVE_RENDERED {result}");
                    webview.snapshot(webkit2gtk::SnapshotRegion::Visible,webkit2gtk::SnapshotOptions::NONE,None::<&gio::Cancellable>,move|result|{
                        let surface=result.unwrap();let mut file=std::fs::File::create("native-capture.png").unwrap();surface.write_to_png(&mut file).unwrap();
                        println!("P11_NATIVE_CAPTURE_RETAINED");
                        if crash{std::process::abort();}else{handle2.exit(0);}
                    });
                });
            }).unwrap();
        });Ok(())
    }).run(tauri::generate_context!("project-fixture.conf.json")).expect("native fixture runtime");
}
#[cfg(not(feature = "visual-test"))]
fn main() {
    eprintln!("Build this fixture with --features visual-test");
    std::process::exit(2);
}
