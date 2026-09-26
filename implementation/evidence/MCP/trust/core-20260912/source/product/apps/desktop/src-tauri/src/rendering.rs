//! Follow the display clock instead of WebKit's compatibility preference for ~60 FPS.
//! Resolve the newer feature API at runtime so older supported WebKit builds still launch.
use std::ffi::{CStr, c_void};
use tauri::Manager;
use webkit2gtk::{SettingsExt, WebViewExt, glib::object::ObjectType};

type Ptr = *mut c_void;
unsafe fn symbol<T: Copy>(name: &CStr) -> Option<T> {
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if address.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute_copy(&address) })
    }
}
fn follow_display(settings: &webkit2gtk::Settings) -> Option<bool> {
    // These signatures are the public WebKitFeature API introduced in WebKitGTK 2.46.
    // All calls run on GTK's main thread; the feature list owns the borrowed feature.
    unsafe {
        let all: unsafe extern "C" fn() -> Ptr = symbol(c"webkit_settings_get_all_features")?;
        let length: unsafe extern "C" fn(Ptr) -> usize = symbol(c"webkit_feature_list_get_length")?;
        let item: unsafe extern "C" fn(Ptr, usize) -> Ptr = symbol(c"webkit_feature_list_get")?;
        let identifier: unsafe extern "C" fn(Ptr) -> *const libc::c_char =
            symbol(c"webkit_feature_get_identifier")?;
        let set: unsafe extern "C" fn(Ptr, Ptr, libc::c_int) =
            symbol(c"webkit_settings_set_feature_enabled")?;
        let get: unsafe extern "C" fn(Ptr, Ptr) -> libc::c_int =
            symbol(c"webkit_settings_get_feature_enabled")?;
        let release: unsafe extern "C" fn(Ptr) = symbol(c"webkit_feature_list_unref")?;
        let list = all();
        if list.is_null() {
            return None;
        }
        let mut result = None;
        for index in 0..length(list) {
            let feature = item(list, index);
            if feature.is_null() {
                continue;
            }
            let name = identifier(feature);
            if name.is_null() {
                continue;
            }
            if CStr::from_ptr(name) == c"PreferPageRenderingUpdatesNear60FPS" {
                let pointer = settings.as_ptr().cast();
                set(pointer, feature, 0);
                result = Some(get(pointer, feature) == 0);
                break;
            }
        }
        release(list);
        result
    }
}
pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.with_webview(|webview| {
            if let Some(settings) = webview.inner().settings() {
                settings.set_hardware_acceleration_policy(webkit2gtk::HardwareAccelerationPolicy::Always);
                settings.set_enable_smooth_scrolling(true);
                match follow_display(&settings) {
                    Some(true) => eprintln!("AG1 rendering: display-paced updates; WebKit near-60-FPS preference disabled"),
                    _ => eprintln!("AG1 rendering: this WebKit build does not expose the display-rate preference"),
                }
            }
        })?;
    }
    Ok(())
}
