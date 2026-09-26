//! Native MCP OAuth consent and external-browser launch. No authorization URL,
//! state, code, verifier or token passes through WebKit command arguments.
use ag1_core::{DomainError, SecretBytes};
use ag1_platform::{
    mcp::{
        definitions::Configuration,
        manager::{OAuthEntry, OAuthReview, SessionLease},
        oauth::{
            Network,
            login::{Browser, ClientRegistration},
        },
    },
    provider::http::{CancelSource, cancellation},
    resource_fs::ScanControl,
};
use gtk::prelude::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub struct Entry {
    window: tauri::WebviewWindow,
    active: Mutex<()>,
    network: Network,
}
impl Entry {
    pub fn new(window: tauri::WebviewWindow) -> Self {
        Self {
            window,
            active: Mutex::new(()),
            network: Network::default(),
        }
    }
    fn schedule(&self, task: impl FnOnce() + Send + 'static) -> Result<(), DomainError> {
        self.window
            .run_on_main_thread(task)
            .map_err(|_| DomainError::Unavailable("Cannot open MCP authentication dialog".into()))
    }
    fn prompt(
        &self,
        body: String,
        issuers: Option<Vec<String>>,
        control: &ScanControl<'_>,
    ) -> Result<(String, String), DomainError> {
        control.check()?;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let closed = Arc::new(AtomicBool::new(false));
        let _close = Close(closed.clone());
        let parent = self.window.clone();
        self.schedule(move || {
            if closed.load(Ordering::Acquire) {
                return;
            }
            let Ok(parent) = parent.gtk_window() else {
                let _ = tx.send(Err(DomainError::Unavailable(
                    "MCP dialog parent unavailable".into(),
                )));
                return;
            };
            let dialog = gtk::Dialog::with_buttons(
                Some("AG1 — MCP sign-in"),
                Some(&parent),
                gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
                &[
                    ("Cancel", gtk::ResponseType::Cancel),
                    ("Continue", gtk::ResponseType::Accept),
                ],
            );
            dialog.set_default_size(650, 500);
            let area = dialog.content_area();
            area.set_spacing(12);
            margins(&area);
            summary(&area, &body);
            let select = gtk::ComboBoxText::new();
            let client = gtk::Entry::new();
            client.set_max_length(4096);
            let choose = issuers.is_some();
            if let Some(issuers) = issuers {
                let label = gtk::Label::new(Some("Authorization server"));
                label.set_xalign(0.0);
                area.pack_start(&label, false, false, 0);
                for issuer in issuers {
                    select.append(Some(&issuer), &issuer);
                }
                select.set_active(Some(0));
                area.pack_start(&select, false, false, 0);
                let label = gtk::Label::new(Some("Registered public client ID"));
                label.set_xalign(0.0);
                area.pack_start(&label, false, false, 0);
                area.pack_start(&client, false, false, 0);
                dialog.set_response_sensitive(gtk::ResponseType::Accept, false);
                let weak = dialog.downgrade();
                client.connect_changed(move |entry| {
                    if let Some(d) = weak.upgrade() {
                        d.set_response_sensitive(
                            gtk::ResponseType::Accept,
                            !entry.text().is_empty(),
                        );
                    }
                });
            }
            let weak = dialog.downgrade();
            let stop = closed.clone();
            let timer = gtk::glib::timeout_add_local(Duration::from_millis(25), move || {
                if let Some(d) = weak.upgrade() {
                    if stop.load(Ordering::Acquire) {
                        d.response(gtk::ResponseType::Cancel);
                        gtk::glib::ControlFlow::Break
                    } else {
                        gtk::glib::ControlFlow::Continue
                    }
                } else {
                    gtk::glib::ControlFlow::Break
                }
            });
            dialog.show_all();
            if choose {
                client.grab_focus();
            }
            gtk::glib::MainContext::default().spawn_local(async move {
                let response = dialog.run_future().await;
                let result =
                    if response == gtk::ResponseType::Accept && !closed.load(Ordering::Acquire) {
                        Ok((
                            select
                                .active_id()
                                .map(|s| s.to_string())
                                .unwrap_or_default(),
                            client.text().to_string(),
                        ))
                    } else {
                        Err(DomainError::Cancelled)
                    };
                dialog.close();
                unsafe {
                    dialog.destroy();
                }
                if !closed.load(Ordering::Acquire) {
                    timer.remove();
                }
                let _ = tx.send(result);
            });
        })?;
        loop {
            control.check()?;
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(r) => {
                    control.check()?;
                    return r;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(DomainError::Cancelled),
            }
        }
    }
    fn progress(&self, body: String, cancel: Arc<CancelSource>) -> Result<Close, DomainError> {
        let closed = Arc::new(AtomicBool::new(false));
        let result = Close(closed.clone());
        let parent = self.window.clone();
        self.schedule(move || {
            if closed.load(Ordering::Acquire) {
                return;
            }
            let Ok(parent) = parent.gtk_window() else {
                cancel.cancel();
                return;
            };
            let dialog = gtk::Dialog::with_buttons(
                Some("AG1 — MCP authentication"),
                Some(&parent),
                gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
                &[("Cancel", gtk::ResponseType::Cancel)],
            );
            dialog.set_default_size(610, 320);
            let area = dialog.content_area();
            margins(&area);
            summary(&area, &body);
            let weak = dialog.downgrade();
            let done = closed.clone();
            let timer = gtk::glib::timeout_add_local(Duration::from_millis(25), move || {
                if let Some(d) = weak.upgrade() {
                    if done.load(Ordering::Acquire) {
                        d.response(gtk::ResponseType::Close);
                        gtk::glib::ControlFlow::Break
                    } else {
                        gtk::glib::ControlFlow::Continue
                    }
                } else {
                    gtk::glib::ControlFlow::Break
                }
            });
            dialog.show_all();
            gtk::glib::MainContext::default().spawn_local(async move {
                let _ = dialog.run_future().await;
                if !closed.load(Ordering::Acquire) {
                    cancel.cancel();
                    timer.remove();
                }
                dialog.close();
                unsafe {
                    dialog.destroy();
                }
            });
        })?;
        Ok(result)
    }
    async fn watch<T>(
        &self,
        control: &ScanControl<'_>,
        future: impl std::future::Future<Output = Result<T, DomainError>>,
    ) -> Result<T, DomainError> {
        control.check()?;
        tokio::pin!(future);
        let mut timer = tokio::time::interval(Duration::from_millis(25));
        loop {
            tokio::select! {biased;_=timer.tick()=>control.check()?,result=&mut future=>{control.check()?;return result;}}
        }
    }
}
struct Close(Arc<AtomicBool>);
impl Drop for Close {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
fn margins(area: &gtk::Box) {
    area.set_margin_start(20);
    area.set_margin_end(20);
    area.set_margin_top(16);
    area.set_margin_bottom(16);
}
fn summary(area: &gtk::Box, body: &str) {
    let label = gtk::Label::new(Some(body));
    label.set_line_wrap(true);
    label.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(78);
    label.set_xalign(0.0);
    label.set_selectable(true);
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.add(&label);
    area.pack_start(&scroll, true, true, 0);
}
impl Browser for Entry {
    fn open_authorization(&self, url: &SecretBytes) -> Result<(), DomainError> {
        let url = SecretBytes::new(url.expose().to_vec());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let closed = Arc::new(AtomicBool::new(false));
        let _close = Close(closed.clone());
        self.schedule(move || {
            if closed.load(Ordering::Acquire) {
                return;
            }
            let result = std::str::from_utf8(url.expose())
                .map_err(|_| DomainError::InvalidInput("MCP authorization URL encoding".into()))
                .and_then(|url| {
                    gtk::gio::AppInfo::launch_default_for_uri(
                        url,
                        None::<&gtk::gio::AppLaunchContext>,
                    )
                    .map_err(|_| {
                        DomainError::Unavailable(
                            "Cannot open the system browser for MCP sign-in".into(),
                        )
                    })
                });
            let _ = tx.send(result);
        })?;
        rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| DomainError::TimedOut)?
    }
}
impl OAuthEntry for Entry {
    fn enter(
        &self,
        session: &SessionLease,
        review: OAuthReview,
        refresh: bool,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError> {
        let _active = self.active.try_lock().map_err(|_| DomainError::QueueFull)?;
        control.check()?;
        // Dedicated owned worker avoids nesting a Tokio runtime in the native
        // bridge executor. The calling operation joins it before returning.
        std::thread::scope(|scope| {
            scope.spawn(|| {
            let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|DomainError::Unavailable("MCP sign-in worker unavailable".into()))?;
            runtime.block_on(async {
                let (source,mut cancel)=cancellation();let source=Arc::new(source);
                let request=review.request().clone();
                let identity=format!("Server: {}\nServer ID: {}",request.server_name,request.server_id);
                let Configuration::Http {url,..}=&request.configuration else {return Err(DomainError::PermissionDenied)};
                let attempt=if refresh {
                    let auth=session.manager().authentication(&request.server_id)?.ok_or(DomainError::PermissionDenied)?;
                    self.prompt(format!("{identity}\n\nRefresh this server's saved OAuth credentials? Existing connections will stop. Connect again after refresh; pending operations are not replayed.\n\n{}",serde_json::to_string_pretty(&auth.binding.target).map_err(|_|DomainError::Conflict)?),None,control)?;
                    session.begin_oauth_refresh(review)?
                } else {
                    let progress=self.progress(format!("{identity}\n\nReading authorization metadata for {url}."),source.clone())?;
                    let resource=self.watch(control,self.network.discover(url,&mut cancel)).await?;drop(progress);
                    let (issuer,client)=self.prompt(format!("{identity}\n\nMCP endpoint: {url}\nOAuth resource: {}\n\nChoose the authorization server and enter an existing public client ID registered for AG1. The native callback is http://127.0.0.1:<ephemeral-port>/ag1/mcp/oauth/callback (IPv6 loopback fallback). The server must support S256 PKCE and public clients.\n\nAutomatic registration and confidential client secrets are not supported by this flow.",resource.view().resource),Some(resource.view().authorization_servers.clone()),control)?;
                    let progress=self.progress(format!("{identity}\n\nChecking authorization server {issuer}."),source.clone())?;
                    let auth=self.watch(control,self.network.authorization(&resource,&issuer,&mut cancel)).await?;drop(progress);
                    auth.check_code_flow()?;
                    let registration=ClientRegistration::pre_registered(&issuer,&client)?;
                    self.prompt(format!("{identity}\n\nMCP endpoint: {url}\nOAuth resource: {}\nAuthorization server: {}\nClient ID: {}\nBrowser destination: {}\nToken destination: {}\n\nRequested scopes:\n{}\n\nContinue opens your system browser for consent. Credentials stay in the MCP private store and are sent only to the bound destinations. Previous server connections stop. Sign-in does not enable this server or approve tool calls. Account identity is unavailable unless independently verified.",auth.resource().resource,auth.view().issuer,registration.client_id(),auth.view().authorization_endpoint,auth.view().token_endpoint,auth.resource().scopes.iter().cloned().collect::<Vec<_>>().join("\n")),None,control)?;
                    session.begin_oauth(review,auth,registration)?
                };
                let _progress=self.progress(format!("{identity}\n\n{}\n\nYou can cancel here. Return to AG1 after completing browser consent.",if refresh {"Refreshing credentials."} else {"Complete sign-in in your system browser."}),source.clone())?;
                let (attempt,tokens)=self.watch(control,attempt.perform(&self.network,Some(self),&mut cancel)).await?;
                control.check()?;session.finish_oauth(attempt,tokens)
            })
        }).join().map_err(|_|DomainError::Unavailable("MCP authentication worker failed".into()))?
        })
    }
}
