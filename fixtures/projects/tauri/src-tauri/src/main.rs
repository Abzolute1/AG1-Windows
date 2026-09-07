#[tauri::command]
fn fixture_done(app: tauri::AppHandle, count: String) {
    assert_eq!(count, "1");
    println!("P11_TAURI_DEV_INTERACTION count={count}");
    app.exit(0);
}
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![fixture_done])
        .run(tauri::generate_context!()).expect("Tauri target runtime");
}
