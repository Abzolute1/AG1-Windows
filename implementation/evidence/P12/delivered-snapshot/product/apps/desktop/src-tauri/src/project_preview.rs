//! Host-only interactive preview. P12 supplies its client actions; P13 owns
//! desktop authority transfer. No renderer URL becomes an AG1 capability.
use ag1_platform::projects::{Runtime, preview::Grant};
use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
pub struct ProjectPreview {
    pub grant: Grant,
    window: WebviewWindow,
}
impl ProjectPreview {
    pub fn open(app: &tauri::AppHandle, runtime: &Runtime) -> Result<Self, String> {
        let grant = runtime.preview_grant().map_err(|e| format!("{e:?}"))?;
        let policy = grant.clone();
        let label = format!("project-preview-{}", grant.operation);
        let url = grant.origin.parse().map_err(|_| "Invalid preview origin")?;
        let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
            .title("Project preview")
            .inner_size(1024., 768.)
            .incognito(true)
            .on_navigation(move |url| policy.allows(&policy.operation, url.as_str()))
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { grant, window })
    }
    pub fn window(&self) -> &WebviewWindow {
        &self.window
    }
    pub fn focus(&self, runtime: &Runtime) -> Result<(), String> {
        runtime
            .verify_preview(&self.grant)
            .map_err(|e| format!("{e:?}"))?;
        self.window.set_focus().map_err(|e| e.to_string())
    }
    pub fn close(&mut self) -> Result<(), String> {
        self.grant.close();
        if self
            .window
            .app_handle()
            .get_webview_window(self.window.label())
            .is_none()
        {
            return Ok(());
        }
        self.window.close().map_err(|e| e.to_string())
    }
}
