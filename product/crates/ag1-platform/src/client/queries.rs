use super::*;
use crate::workspace::preserve_text;
use sha2::{Digest, Sha256};
impl Host {
    pub(super) fn query(&self, command: &w::Command) -> Result<Option<Value>> {
        let selected = self.registry.selected()?;
        let session = selected.services.runtime();
        Ok(Some(match command {
            w::Command::Runtime {action,args} if action=="client_effect_result"=>self.client_effect_result(args)?,
            w::Command::Runtime {action,args} if action.starts_with("terminal_")=>self.terminal_command(action.trim_start_matches("terminal_"),args)?,
            w::Command::Runtime {action,args} if action=="detail_page"=>self.detail_page(args)?,
            w::Command::Runtime {action,..} if action=="resource_actions"=>selected.services.tools().client_resources().ok_or(DomainError::NotFound)?.catalog(),
            w::Command::Runtime {action,args} if action=="extension_answer"=>self.ui.answer(&self.commands.snapshot()?.scope,args)?,
            w::Command::Runtime {action,..} if action=="extension_ui"=>self.ui.view()?,
            w::Command::Runtime {action,..} if action=="extension_settings"=>{
                let extension=selected.services.tools().client_extensions().ok_or(DomainError::NotFound)?;let snapshot=self.commands.snapshot()?;
                extension.client_settings(&ag1_core::tools::registry::Scope {workspace:snapshot.scope.workspace.as_str().into(),session:snapshot.scope.session.as_str().into(),epoch:session.context_epoch()?,revision:session.journal_revision()?},&crate::resource_fs::ScanControl {cancel:&AtomicBool::new(false),deadline:std::time::Instant::now()+Duration::from_secs(10)})?
            },
            w::Command::Runtime {action,..} if action=="extension_catalog"=>session_catalog(&selected)?,
            w::Command::Artifacts {action,args} if action.starts_with("upload_")=>self.upload(action,args)?,
            w::Command::Project {action,args}=>self.project_command(action,args)?,
            w::Command::Runtime {action,..} if action=="resources"=>{
                let mut catalog=json!(self.resources()?.catalog);
                for kind in ["skills","prompts"] {
                    for item in catalog[kind].as_array_mut().into_iter().flatten() {
                        let path=item["filePath"].as_str().or_else(||item["path"].as_str()).and_then(|p|Path::new(p).strip_prefix(self.workspace.root()).ok()).map(|p|p.to_string_lossy().into_owned());
                        if let Some(path)=path { item["workspace_path"]=json!(path); }
                    }
                }
                catalog
            },
            w::Command::Files {action,args}=>match action.as_str() {
                "list"=>json!(self.workspace.scan(1000)?),
                "diff"=>{let t=self.config.tooling.as_ref().ok_or_else(||DomainError::Unavailable("Configure the native Git helper".into()))?;let spools=self.state.join("diff");fs::create_dir_all(&spools).map_err(crate::io_error)?;let diff=crate::git::GitService::new(&t.process_host,&t.git,&spools)?.status_diff(&self.workspace,None)?;json!({"view":format!("HEAD {}\n{}\n{}",diff.head,String::from_utf8_lossy(&diff.status),String::from_utf8_lossy(&diff.diff))})},
                "read"|"save"=>{
                    let path=args["path"].as_str().ok_or_else(||DomainError::InvalidInput("file path".into()))?;
                    let current=self.workspace.snapshot(path,32768)?;
                    let token=format!("{:x}",Sha256::digest(format!("{:?}",current.revision).as_bytes()));
                    if action=="save" {
                        if args["revision"]!=token {return Err(DomainError::Conflict);}
                        let text=args["text"].as_str().ok_or_else(||DomainError::InvalidInput("file text".into()))?;
                        let bytes=preserve_text(&current,text)?;
                        if bytes.len()>32768 {return Err(DomainError::LimitExceeded("editor save".into()));}
                        self.workspace.replace(path,&current.revision,&bytes)?;
                        return self.query(&w::Command::Files {action:"read".into(),args:json!({"path":path})});
                    }
                    json!({"path":path,"text":String::from_utf8(current.bytes).map_err(|_|DomainError::InvalidInput("binary file; use artifacts".into()))?,"revision":token})
                },
                _=>return Err(DomainError::Unavailable(format!("files.{action}"))),
            },
            w::Command::Model {action,..} if action=="status"=>json!({"models":self.config.models.iter().map(|m|json!({"id":m["id"],"profile":{"provider":m["provider"],"runtime":m["runtime"],"conflict":m["runtimeConflict"],"contextWindow":m["contextWindow"]}})).collect::<Vec<_>>(),"tuning":false}),
            w::Command::Model {action,..} if action=="tune"=>return Err(DomainError::Unavailable("Tuning requires a separately reviewed P06 tuning operation; client open/select never tunes".into())),
            w::Command::Diagnostics {action,..} if action=="status"=>json!(self.view()?.details),
            w::Command::Session {action,..} if action=="list"=>json!(self.store.client_sessions(1000)?),
            w::Command::Session {action,args} if action=="history"=>{
                use ag1_core::storage::StoragePort;
                let journal=self.store.journal_snapshot(&SessionId::new(self.commands.snapshot()?.scope.session.as_str())?)?;
                let offset=args["offset"].as_u64().unwrap_or(0) as usize;
                let entries=journal.entries.iter().skip(offset).take(50).map(|entry| {
                    let value:Value=serde_json::from_slice(&entry.normalized).map_err(json_error)?;
                    let preview=projection::message(&value);
                    Ok(json!({"id":entry.id,"parent":entry.parent,"kind":value["type"],"role":value["message"]["role"],"preview":preview.as_ref().and_then(|m|m.blocks.iter().find_map(|b|if let w::MessageBlock::Text {text}=b{Some(text.chars().take(160).collect::<String>())}else{None}))}))
                }).collect::<Result<Vec<_>>>()?;
                json!({"entries":entries,"total":journal.entries.len(),"next":if offset+entries.len()<journal.entries.len(){Some(offset+entries.len())}else{None}})
            },
            w::Command::Session {action,args} if action=="user_text"=>self.user_text(args)?,
            w::Command::Session {action,args} if action=="entry"=>{
                use base64::{Engine,engine::general_purpose::STANDARD};
                let records=session.client_records()?;
                let entry=records.iter().find(|r|r["id"]==args["id"]).ok_or(DomainError::NotFound)?;
                let bytes=serde_json::to_vec_pretty(entry).map_err(json_error)?;
                let offset=args["offset"].as_str().unwrap_or("0").parse::<usize>().map_err(|_|DomainError::InvalidInput("entry byte offset".into()))?;
                if offset>bytes.len() {return Err(DomainError::InvalidInput("entry byte offset".into()));}
                let end=(offset+32768).min(bytes.len());
                json!({"base64":STANDARD.encode(&bytes[offset..end]),"next":if end<bytes.len(){Some(end.to_string())}else{None},"sha256":ag1_storage::digest(&bytes)})
            },
            w::Command::Session {action,args} if action=="messages"=>{
                let records=session.client_records()?;
                let rows:Vec<_>=records.iter().filter_map(projection::message).collect();
                if let Some(before) = args["before"].as_u64() {
                    let before=(before as usize).min(rows.len());
                    let mut page=Vec::new();let mut bytes=0;
                    for row in rows[..before].iter().rev().take(100) {let n=serde_json::to_vec(row).map_err(json_error)?.len();if bytes+n>32000 {break;}bytes+=n;page.push(row);}
                    let start=before-page.len();page.reverse();
                    json!({"messages":page,"start":start,"next_before":if start>0{Some(start)}else{None},"total":rows.len()})
                } else {
                    let offset=args["offset"].as_u64().unwrap_or(0) as usize;
                    if offset>rows.len() {return Err(DomainError::InvalidInput("message offset".into()));}
                    let mut page=Vec::new();let mut bytes=0;
                    for row in rows.iter().skip(offset).take(100) {let n=serde_json::to_vec(row).map_err(json_error)?.len();if bytes+n>32000 {break;}bytes+=n;page.push(row);}
                    json!({"messages":page,"next":if offset+page.len()<rows.len(){Some(offset+page.len())}else{None},"total":rows.len()})
                }
            },
            w::Command::Session {action,args} if action=="export"=>{
                let records=session.client_records()?;let html=export::full_html(&records)?;
                let default=format!("session-{}-{}.html",self.commands.snapshot()?.scope.session.as_str(),session.journal_revision()?);
                let path=args["path"].as_str().unwrap_or(&default);
                if !path.ends_with(".html") {return Err(DomainError::InvalidInput("HTML export filename".into()));}
                let revision=self.workspace.create_checked(path,html.as_bytes(),false,||Ok(()))?;
                json!({"path":path,"sha256":revision.sha256,"bytes":html.len(),"messages":records.iter().filter(|r|r["type"]=="message").count(),"notice":"Complete branch text; binary attachments remain in journal custody"})
            },
            w::Command::Task {action,..} if action=="view"=>self.view()?.details.task,
            w::Command::Plan {action,..} if action=="view"=>json!(session.plan_snapshot(30)?.plans),
            w::Command::Artifacts {action,args} if action=="list"=>{
                let before=args.get("before").filter(|v|!v.is_null()).map(|v|Ok::<_,DomainError>(ag1_storage::CatalogPosition {created_at:v["created_at"].as_str().ok_or(DomainError::NotFound)?.into(),id:v["id"].as_str().ok_or(DomainError::NotFound)?.into(),origin:v["origin"].as_str().ok_or(DomainError::NotFound)?.into()})).transpose()?;
                let rows=self.store.artifact_catalog(self.commands.snapshot()?.scope.session.as_str(),before.as_ref(),20,args["kind"].as_str())?;
                let next=rows.last().filter(|_|rows.len()==20).map(|r|json!({"created_at":r.position.created_at,"id":r.position.id,"origin":r.position.origin}));
                json!({"artifacts":rows.iter().map(|r|json!({"id":r.position.id,"kind":r.record["kind"],"mime_type":r.record["mimeType"],"bytes":r.record["bytes"],"sha256":r.record["sha256"],"created_at":r.position.created_at})).collect::<Vec<_>>(),"next":next})
            },
            w::Command::Memory {action,..} if action=="view"=>self.view()?.details.memory,
            w::Command::Runtime {action,..} if action=="status"=>self.view()?.details.activity,
            _=>return Ok(None),
        }))
    }
}

fn session_catalog(selected: &crate::session::registry::Selection) -> Result<Value> {
    Ok(selected.services.tools().client_extensions().map(|e|e.catalog().clone()).unwrap_or_else(||json!({"commands":[],"shortcuts":[],"tools":[],"status":"No admitted extension host; configure explicit host trust"})))
}
