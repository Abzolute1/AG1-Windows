//! Follow the display clock instead of WebKit's compatibility preference for ~60 FPS.
//! Resolve the newer feature API at runtime so older supported WebKit builds still launch.
use std::ffi::{CStr, c_void};
use tauri::Manager;
use webkit2gtk::{SettingsExt, WebViewExt, glib::object::ObjectType};

unsafe extern "C" {
    fn ag1_frame_clock_initialize(candidate: libc::c_int);
    fn ag1_frame_clock_attach(widget: *mut c_void);
}

/// Called before GTK, workers or runtime threads start. Explicit user graphics
/// overrides remain available; prefer the compositor's native Wayland clock.
pub fn configure_environment() {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let nvidia = std::fs::read_dir("/sys/class/drm").ok().into_iter().flatten().flatten()
        .take(64).any(|entry| std::fs::read_to_string(entry.path().join("device/vendor")).is_ok_and(|v| v.trim() == "0x10de"));
    unsafe {
        if wayland && std::env::var_os("GDK_BACKEND").is_none() {
            std::env::set_var("GDK_BACKEND", "wayland,x11");
        }
        if nvidia && std::env::var_os("WEBKIT_DMABUF_RENDERER_FORCE_SHM").is_none() && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            // NVIDIA's DMA-BUF import path can disconnect the Wayland display.
            // Shared buffers retain WebKit compositing without that import path.
            std::env::set_var("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1");
        }
        let candidate = wayland && std::env::var("GDK_BACKEND").is_ok_and(|v| v.split(',').next() == Some("wayland"))
            && std::env::var("AG1_DISABLE_DISPLAY_CLOCK_FALLBACK").as_deref() != Ok("1");
        ag1_frame_clock_initialize(i32::from(candidate));
    }
}

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
        // Hyprland users manage window chrome in the compositor (e.g. hyprbars).
        // Tao adds a GTK header on Wayland, which otherwise duplicates that bar.
        // Keep native decorations elsewhere and allow an explicit local override.
        let compositor_chrome = std::env::var("XDG_CURRENT_DESKTOP")
            .is_ok_and(|value| value.split(':').any(|name| name.eq_ignore_ascii_case("Hyprland")));
        let decorations = match std::env::var("AG1_NATIVE_TITLEBAR").as_deref() {
            Ok("1") => Some(true),
            Ok("0") => Some(false),
            _ if compositor_chrome => Some(false),
            _ => None,
        };
        if let Some(decorations) = decorations {
            window.set_decorations(decorations)?;
        }
        window.with_webview(|webview| {
            unsafe { ag1_frame_clock_attach(webview.inner().as_ptr().cast()); }
            if let Some(settings) = webview.inner().settings() {
                settings.set_hardware_acceleration_policy(webkit2gtk::HardwareAccelerationPolicy::Always);
                settings.set_enable_smooth_scrolling(true);
                match follow_display(&settings) {
                    Some(true) => eprintln!("AG1 rendering: WebKit near-60-FPS preference disabled; cadence depends on the active display clock"),
                    _ => eprintln!("AG1 rendering: this WebKit build does not expose the display-rate preference"),
                }
            }
        })?;
    }
    Ok(())
}
