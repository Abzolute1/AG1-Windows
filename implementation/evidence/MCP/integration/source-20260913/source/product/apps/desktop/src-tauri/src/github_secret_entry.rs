//! Password entry is GTK-native, outside WebKit and the conversation bridge.
use ag1_platform::{github::{operations::Operation,secrets::{PrivateEntry,Secret,MAX_BYTES},service::{Account,ApiError,Result,check}},resource_fs::ScanControl};
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
use gtk::prelude::*;
pub struct Entry(pub tauri::WebviewWindow);
impl PrivateEntry for Entry {
    fn enter(&self,account:&Account,operation:&Operation,control:&ScanControl<'_>)->Result<Secret>{
        check(control)?;
        let (sender,receiver)=std::sync::mpsc::sync_channel(1);
        let cancel=Arc::new(AtomicBool::new(false));let closed=cancel.clone();
        struct Close(Arc<AtomicBool>);impl Drop for Close {fn drop(&mut self){self.0.store(true,Ordering::Release);}}
        let _close=Close(cancel);
        let parent=self.0.clone();let account=account.public();let metadata=serde_json::to_value(operation).map_err(|_|ApiError::new("private_entry","Invalid secret target."))?;let endpoint=metadata["endpoint"].as_str().unwrap_or("").to_owned();let fields=metadata["body"].clone();
        let access=match fields["visibility"].as_str(){Some("all")=>"All organization repositories".into(),Some("private")=>"Private organization repositories".into(),_ if fields["selected_repository_ids"].is_array()=>format!("Selected repository IDs: {}",fields["selected_repository_ids"]),_=>"Uses this destination’s permissions".into()};
        self.0.run_on_main_thread(move||{
            if closed.load(Ordering::Acquire){return;}
            let parent=match parent.gtk_window(){Ok(p)=>p,Err(_)=>{let _=sender.send(Err(ApiError::new("private_entry","Cannot open private secret entry.")));return;}};
            let dialog=gtk::Dialog::with_buttons(Some("AG1 — GitHub secret"),Some(&parent),gtk::DialogFlags::MODAL|gtk::DialogFlags::DESTROY_WITH_PARENT,&[("Cancel",gtk::ResponseType::Cancel),("Encrypt and save",gtk::ResponseType::Accept)]);
            dialog.set_default_size(600,260);dialog.set_resizable(false);
            let area=dialog.content_area();area.set_spacing(12);area.set_margin_start(20);area.set_margin_end(20);area.set_margin_top(16);area.set_margin_bottom(16);
            let label=gtk::Label::new(Some(&format!("Account: @{} on github.com\nTarget: {}\nAccess: {}\n\nEnter the value below. Saving replaces an existing secret with this name.\nThe encrypted value is sent to GitHub; it is not shown in your conversation.",account["login"].as_str().unwrap_or(""),endpoint,access)));label.set_line_wrap(true);label.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);label.set_max_width_chars(72);label.set_xalign(0.0);
            let summary=gtk::ScrolledWindow::new(None::<&gtk::Adjustment>,None::<&gtk::Adjustment>);summary.set_policy(gtk::PolicyType::Never,gtk::PolicyType::Automatic);summary.set_min_content_height(130);summary.set_max_content_height(170);summary.set_propagate_natural_height(true);summary.add(&label);area.pack_start(&summary,false,false,0);
            let entry=gtk::Entry::new();entry.set_visibility(false);entry.set_truncate_multiline(false);entry.set_input_purpose(gtk::InputPurpose::Password);entry.set_max_length((MAX_BYTES+1) as i32);entry.set_activates_default(true);entry.set_placeholder_text(Some("Secret value (up to 48 KiB)"));area.pack_start(&entry,false,false,0);
            dialog.set_default_response(gtk::ResponseType::Accept);dialog.set_response_sensitive(gtk::ResponseType::Accept,false);
            let weak=dialog.downgrade();entry.connect_changed(move|entry|{if let Some(dialog)=weak.upgrade(){let len=entry.text_length();dialog.set_response_sensitive(gtk::ResponseType::Accept,len>0&&len as usize<=MAX_BYTES);}});
            let weak=dialog.downgrade();let cancelled=closed.clone();let timer=gtk::glib::timeout_add_local(std::time::Duration::from_millis(25),move||{
                if let Some(dialog)=weak.upgrade(){if cancelled.load(Ordering::Acquire){dialog.response(gtk::ResponseType::Cancel);return gtk::glib::ControlFlow::Break;}gtk::glib::ControlFlow::Continue}else{gtk::glib::ControlFlow::Break}
            });
            dialog.show_all();entry.grab_focus();
            gtk::glib::MainContext::default().spawn_local(async move{
                let response=dialog.run_future().await;
                let result=if response==gtk::ResponseType::Accept&&!closed.load(Ordering::Acquire){
                    // Copy directly from GTK into the opaque native buffer;
                    // avoid an intermediate serializable Rust String.
                    let text=unsafe{gtk::ffi::gtk_entry_get_text(entry.as_ptr())};
                    Secret::new(unsafe{std::ffi::CStr::from_ptr(text)}.to_bytes().to_vec())
                }else{Err(ApiError::new("cancelled","Secret entry cancelled. No secret was sent."))};
                entry.set_text("");dialog.close();unsafe{dialog.destroy();}
                // A cancelled timer may already have removed its own source.
                if !closed.load(Ordering::Acquire){timer.remove();}
                let _=sender.send(result);
            });
        }).map_err(|_|ApiError::new("private_entry","Cannot schedule private secret entry."))?;
        loop {check(control)?;match receiver.recv_timeout(std::time::Duration::from_millis(20)){Ok(result)=>{check(control)?;return result;},Err(std::sync::mpsc::RecvTimeoutError::Timeout)=>{},Err(_)=>return Err(ApiError::new("private_entry","Private secret entry closed."))}}
    }
}
