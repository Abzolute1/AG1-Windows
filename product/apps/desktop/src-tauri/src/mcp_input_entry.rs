//! Native choice forms. No opaque continuation state or responses cross WebKit.
use ag1_core::DomainError;
use ag1_platform::{
    mcp::input::{self, Answer, Choices, Form},
    resource_fs::ScanControl,
};
use gtk::prelude::*;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
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
impl input::Entry for Entry {
    fn enter(&self, form: &Form, control: &ScanControl<'_>) -> Result<Answer, DomainError> {
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
        let form = form.clone();
        self.window.run_on_main_thread(move||{
            if closed.load(Ordering::Acquire){return;}
            let parent=match parent.gtk_window(){Ok(p)=>p,Err(_)=>{let _=sender.send(Err(DomainError::Unavailable("Cannot open MCP input form".into())));return;}};
            let dialog=gtk::Dialog::with_buttons(Some("AG1 — MCP needs your input"),Some(&parent),gtk::DialogFlags::MODAL|gtk::DialogFlags::DESTROY_WITH_PARENT,&[("Cancel operation",gtk::ResponseType::Cancel),("Decline",gtk::ResponseType::Reject),("Send answers and continue",gtk::ResponseType::Accept)]);
            dialog.set_default_size(680,600);let area=dialog.content_area();area.set_spacing(12);area.set_margin_start(20);area.set_margin_end(20);area.set_margin_top(16);area.set_margin_bottom(16);
            let label=|text:&str|{let label=gtk::Label::new(Some(text));label.set_xalign(0.0);label.set_line_wrap(true);label.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);label.set_max_width_chars(80);label};
            area.pack_start(&label(&format!("{} · Input round {} of {}\nServer: {}\nCall: {}",form.identity.original_name,form.round,input::MAX_ROUNDS,form.identity.binding.server_id,form.call_id)),false,false,0);
            area.pack_start(&label("This is an untrusted server question for the operation you already approved. Answers return only to that same bound operation. This does not grant access to other tools, servers, folders or credentials."),false,false,0);
            let binding=gtk::Expander::new(Some("Original operation and scope"));let details=label(&serde_json::to_string_pretty(&serde_json::json!({"identity":form.identity,"arguments":form.original_arguments})).unwrap_or_default());
            let detail_scroll=gtk::ScrolledWindow::new(None::<&gtk::Adjustment>,None::<&gtk::Adjustment>);detail_scroll.set_max_content_height(160);detail_scroll.set_propagate_natural_height(true);detail_scroll.add(&details);binding.add(&detail_scroll);area.pack_start(&binding,false,false,0);
            let content=gtk::Box::new(gtk::Orientation::Vertical,10);content.pack_start(&label(&form.message),false,false,0);
            let mut fields=Vec::new();
            for field in &form.fields{
                content.pack_start(&label(&format!("{}{}",field.title,if field.required{" (required)"}else{" (optional)"})),false,false,0);
                if !field.description.is_empty(){content.pack_start(&label(&field.description),false,false,0);}
                let choice=gtk::ComboBoxText::new();choice.append_text(if field.required{"Choose an answer"}else{"Leave unanswered"});
                match &field.choices{Choices::Boolean=>{choice.append_text("Yes");choice.append_text("No");},Choices::Enum(values)=>{for value in values{choice.append_text(value);}}}
                choice.set_active(Some(0));content.pack_start(&choice,false,false,0);fields.push((field.clone(),choice));
            }
            let scroll=gtk::ScrolledWindow::new(None::<&gtk::Adjustment>,None::<&gtk::Adjustment>);scroll.set_policy(gtk::PolicyType::Never,gtk::PolicyType::Automatic);scroll.add(&content);area.pack_start(&scroll,true,true,0);
            let ack=gtk::CheckButton::with_label("I reviewed these answers and want to continue this operation.");area.pack_start(&ack,false,false,0);dialog.set_response_sensitive(gtk::ResponseType::Accept,false);
            for (_,choice) in &fields{
                let all=fields.iter().map(|(f,c)|(f.required,c.downgrade())).collect::<Vec<_>>();let weak=dialog.downgrade();let ack=ack.downgrade();
                choice.connect_changed(move |_|{if let Some(dialog)=weak.upgrade(){dialog.set_response_sensitive(gtk::ResponseType::Accept,ack.upgrade().is_some_and(|a|a.is_active())&&all.iter().all(|(required,c)|!required||c.upgrade().is_some_and(|c|c.active().is_some_and(|i|i>0))));}});
            }
            let all=fields.iter().map(|(f,c)|(f.required,c.downgrade())).collect::<Vec<_>>();let weak=dialog.downgrade();ack.connect_toggled(move|ack|{if let Some(dialog)=weak.upgrade(){dialog.set_response_sensitive(gtk::ResponseType::Accept,ack.is_active()&&all.iter().all(|(required,c)|!required||c.upgrade().is_some_and(|c|c.active().is_some_and(|i|i>0))));}});
            let weak=dialog.downgrade();let cancelled=closed.clone();let timer=gtk::glib::timeout_add_local(Duration::from_millis(25),move||{if let Some(dialog)=weak.upgrade(){if cancelled.load(Ordering::Acquire){dialog.response(gtk::ResponseType::Cancel);gtk::glib::ControlFlow::Break}else{gtk::glib::ControlFlow::Continue}}else{gtk::glib::ControlFlow::Break}});
            dialog.show_all();
            gtk::glib::MainContext::default().spawn_local(async move{
                let response=dialog.run_future().await;
                let answer=if closed.load(Ordering::Acquire){Answer::Cancel}else if response==gtk::ResponseType::Reject{Answer::Decline}else if response==gtk::ResponseType::Accept && ack.is_active(){
                    let mut values=BTreeMap::new();for (field,choice) in fields{let Some(index)=choice.active().filter(|i|*i>0)else{continue;};let value=match field.choices{Choices::Boolean=>serde_json::json!(index==1),Choices::Enum(options)=>match options.get(index as usize-1){Some(value)=>serde_json::json!(value),None=>continue}};values.insert(field.name,value);}Answer::Accept(values)
                }else{Answer::Cancel};
                dialog.close();unsafe{dialog.destroy();}if !closed.load(Ordering::Acquire){timer.remove();}let _=sender.send(Ok(answer));
            });
        }).map_err(|_|DomainError::Unavailable("Cannot schedule MCP input form".into()))?;
        loop {
            control.check()?;
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(answer) => {
                    control.check()?;
                    return answer;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(DomainError::Cancelled),
            }
        }
    }
}
