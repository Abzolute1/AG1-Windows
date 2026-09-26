//! GTK private entry: values never pass through WebKit, bridge arguments or logs.
use ag1_core::{DomainError, SecretBytes};
use ag1_platform::{
    mcp::{
        auth::{MAX_SECRET_BYTES, Material},
        definitions::Configuration,
        manager::{PrivateEntry, PrivateRequest},
    },
    resource_fs::ScanControl,
};
use gtk::prelude::*;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
pub struct Entry {
    window: tauri::WebviewWindow,
    active: Mutex<()>,
}
impl Entry {
    pub fn new(window: tauri::WebviewWindow) -> Self {
        Self {
            window,
            active: Mutex::new(()),
        }
    }
}
impl PrivateEntry for Entry {
    fn enter(
        &self,
        request: &PrivateRequest,
        control: &ScanControl<'_>,
    ) -> Result<Material, DomainError> {
        let _active = self.active.try_lock().map_err(|_| DomainError::QueueFull)?;
        control.check()?;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let closed = Arc::new(AtomicBool::new(false));
        struct Close(Arc<AtomicBool>);
        impl Drop for Close {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _close = Close(closed.clone());
        let parent = self.window.clone();
        let request = request.clone();
        self.window.run_on_main_thread(move|| {
            if closed.load(Ordering::Acquire){return;}
            let parent=match parent.gtk_window(){Ok(p)=>p,Err(_)=>{let _=sender.send(Err(DomainError::Unavailable("Cannot open MCP private entry".into())));return;}};
            let dialog=gtk::Dialog::with_buttons(Some("AG1 — MCP private values"),Some(&parent),gtk::DialogFlags::MODAL|gtk::DialogFlags::DESTROY_WITH_PARENT,&[("Cancel",gtk::ResponseType::Cancel),("Save private values",gtk::ResponseType::Accept)]);
            dialog.set_default_size(640,520);
            let area=dialog.content_area();area.set_spacing(12);area.set_margin_start(20);area.set_margin_end(20);area.set_margin_top(16);area.set_margin_bottom(16);
            let configuration=serde_json::to_string_pretty(&request.configuration).unwrap_or_default();
            let identity=gtk::Label::new(Some(&format!("Server: {}\nServer ID: {}",request.server_name,request.server_id)));identity.set_line_wrap(true);identity.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);identity.set_max_width_chars(75);identity.set_xalign(0.0);area.pack_start(&identity,false,false,0);
            let text=match &request.executable_sha256 {Some(hash)=>format!("{}\nExecutable SHA-256: {}",configuration,hash),None=>configuration};
            let label=gtk::Label::new(Some(&text));label.set_line_wrap(true);label.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);label.set_xalign(0.0);label.set_max_width_chars(75);
            let summary=gtk::ScrolledWindow::new(None::<&gtk::Adjustment>,None::<&gtk::Adjustment>);summary.set_policy(gtk::PolicyType::Never,gtk::PolicyType::Automatic);summary.set_min_content_height(120);summary.set_max_content_height(150);summary.set_propagate_natural_height(true);summary.add(&label);area.pack_start(&summary,false,false,0);
            let mut explanation="Values are stored privately on this computer for this exact configuration. The destination receives them when you later connect. Saving replaces the previous values and disconnects this server. It does not connect or approve tool calls.".to_owned();
            if matches!(&request.configuration,Configuration::Http {..}) {explanation.push_str("\n\nUse a token intended for this MCP endpoint. Account identity is not verified from a manual token.");}
            let disclosure=gtk::Label::new(Some(&explanation));disclosure.set_line_wrap(true);disclosure.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);disclosure.set_max_width_chars(75);disclosure.set_xalign(0.0);area.pack_start(&disclosure,false,false,0);
            let content=gtk::Box::new(gtk::Orientation::Vertical,10);
            let (names,limit,local)=match &request.configuration {Configuration::Local {secret_environment,..}=>(secret_environment.iter().cloned().collect::<Vec<_>>(),4096,true),Configuration::Http {..}=>(vec!["MCP bearer token".into()],MAX_SECRET_BYTES,false)};
            let mut fields=Vec::new();
            for name in names {
                let label=gtk::Label::new(Some(&name));label.set_xalign(0.0);content.pack_start(&label,false,false,0);
                let entry=gtk::Entry::new();entry.set_visibility(false);entry.set_input_purpose(gtk::InputPurpose::Password);entry.set_truncate_multiline(false);entry.set_max_length((limit+1) as i32);entry.set_activates_default(true);
                content.pack_start(&entry,false,false,0);fields.push((name,entry));
            }
            let scroll=gtk::ScrolledWindow::new(None::<&gtk::Adjustment>,None::<&gtk::Adjustment>);scroll.set_policy(gtk::PolicyType::Never,gtk::PolicyType::Automatic);scroll.add(&content);area.pack_start(&scroll,true,true,0);
            dialog.set_default_response(gtk::ResponseType::Accept);dialog.set_response_sensitive(gtk::ResponseType::Accept,false);
            for (_,entry) in &fields {
                let entries=fields.iter().map(|(_,e)|e.downgrade()).collect::<Vec<_>>();let weak=dialog.downgrade();
                entry.connect_changed(move |_| {if let Some(dialog)=weak.upgrade(){dialog.set_response_sensitive(gtk::ResponseType::Accept,entries.iter().all(|weak|weak.upgrade().is_some_and(|e|{let pointer=unsafe{gtk::ffi::gtk_entry_get_text(e.as_ptr())};let n=unsafe{std::ffi::CStr::from_ptr(pointer)}.to_bytes().len();n>0&&n<=limit})));}});
            }
            let weak=dialog.downgrade();let cancelled=closed.clone();
            let timer=gtk::glib::timeout_add_local(std::time::Duration::from_millis(25),move||{if let Some(dialog)=weak.upgrade(){if cancelled.load(Ordering::Acquire){dialog.response(gtk::ResponseType::Cancel);gtk::glib::ControlFlow::Break}else{gtk::glib::ControlFlow::Continue}}else{gtk::glib::ControlFlow::Break}});
            dialog.show_all();if let Some((_,entry))=fields.first(){entry.grab_focus();}
            gtk::glib::MainContext::default().spawn_local(async move {
                let response=dialog.run_future().await;
                let result=if response==gtk::ResponseType::Accept&&!closed.load(Ordering::Acquire){
                    let mut values=BTreeMap::new();
                    for (name,entry) in &fields {
                        let text=unsafe{gtk::ffi::gtk_entry_get_text(entry.as_ptr())};
                        let bytes=unsafe{std::ffi::CStr::from_ptr(text)}.to_bytes();
                        values.insert(name.clone(),SecretBytes::new(bytes.to_vec()));
                    }
                    if local {Ok(Material::Environment(values))}else{values.remove("MCP bearer token").map(|access|Material::Bearer {access,refresh:None}).ok_or(DomainError::InvalidInput("MCP private entry".into()))}
                }else{Err(DomainError::Cancelled)};
                for (_,entry) in &fields {entry.set_text("");}dialog.close();unsafe{dialog.destroy();}
                if !closed.load(Ordering::Acquire){timer.remove();}let _=sender.send(result);
            });
        }).map_err(|_|DomainError::Unavailable("Cannot schedule MCP private entry".into()))?;
        loop {
            control.check()?;
            match receiver.recv_timeout(std::time::Duration::from_millis(20)) {
                Ok(result) => {
                    control.check()?;
                    return result;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(DomainError::Cancelled),
            }
        }
    }
}
